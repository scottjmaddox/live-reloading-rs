use ::std;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::mpsc::{channel, Receiver};

use ::notify;
use ::notify::{Watcher, RecommendedWatcher};
use ::libloading;
use ::libloading::Library;

use super::ShouldQuit;

#[cfg(unix)]
type Symbol<T> = libloading::os::unix::Symbol<T>;
#[cfg(windows)]
type Symbol<T> = libloading::os::windows::Symbol<T>;

struct AppSym<Host> {
    /// This needs to be present so that the library will be closed on drop
    _lib: Library,
    api: Symbol<*mut internals::ReloadApi<Host>>,
}

// @Todo: Flesh out this documentation
/// A `Reloadable` represents a handle to library that can be live reloaded.
pub struct Reloadable<Host> {
    path: PathBuf,
    sym: Option<AppSym<Host>>,
    state: Vec<u64>,
    _watcher: RecommendedWatcher,
    rx: Receiver<notify::DebouncedEvent>,
    host: Host,
}

/// The errors that can occur while working with a `Reloadable` object.
#[derive(Debug)]
pub enum Error {
    /// An I/O error occurred while trying to load or reload the library. This
    /// can indicate that the file is missing, or that the library didn't have
    /// the expected `RELOAD_API` symbol.
    // @Diagnostics: Add an error type to distinguish this latter situation
    Io(std::io::Error),
    /// An error occurred while creating the filesystem watcher.
    Watch(notify::Error),
    /// The `Host` type of the host and library don't match.
    MismatchedHost,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::Io(err)
    }
}

impl From<notify::Error> for Error {
    fn from(err: notify::Error) -> Error {
        Error::Watch(err)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        write!(fmt, "{:?}", self)
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::Io(ref err) => err.description(),
            Error::Watch(ref err) => err.description(),
            Error::MismatchedHost => "mismatch between host and library's Host types",
        }
    }
}

impl<Host> AppSym<Host> {
    fn new<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let library = Library::new(path.as_ref())?;
        let api = unsafe {
            library
                .get::<*mut internals::ReloadApi<Host>>(b"RELOAD_API")?
                .into_raw()
        };
        Ok(AppSym {
            _lib: library,
            api: api,
        })
    }
}

impl<Host> Reloadable<Host> {
    /// Create a new Reloadable library.
    ///
    /// This takes the path to a dynamic library containing a `RELOAD_API`
    /// symbol that exports the functions needed for live reloading. In order to
    /// define this symbol in your own reloadable libraries, see the
    /// [`live_reload!`][] macro. This will load the library and initialize a
    /// filesystem watcher pointing to the file in order to know when the
    /// library has changed.
    ///
    /// [`live_reload!`]: macro.live_reload.html
    pub fn new<P: AsRef<Path>>(path: P, host: Host) -> Result<Self, Error> {
        let sym = AppSym::new(&path)?;
        let size = (unsafe { &**sym.api }.size)();
        let (tx, rx) = channel();
        let mut watcher = notify::watcher(tx, Duration::from_secs(1))?;
        let mut new_path = PathBuf::new();
        new_path.push(path);
        watcher.watch(
            new_path.parent().unwrap(),
            notify::RecursiveMode::NonRecursive,
        )?;
        let mut app = Reloadable {
            path: new_path.canonicalize()?,
            sym: Some(sym),
            state: Vec::new(),
            _watcher: watcher,
            rx: rx,
            host: host,
        };
        app.realloc_buffer(size);
        if let Some(AppSym { ref mut api, .. }) = app.sym {
            (unsafe { &***api }.init)(&mut app.host, Self::get_state_ptr(&mut app.state));
        }
        Ok(app)
    }

    /// Reload the library if it has changed, otherwise do nothing.
    ///
    /// This will consult with the filesystem watcher, and if the library has
    /// been recreated or updated, it will reload the library. See
    /// [`reload_now`][] for details on what happens when a library is reloaded.
    ///
    /// [`reload_now`]: struct.Reloadable.html#method.reload_now
    pub fn reload(&mut self) -> Result<(), Error> {
        let mut should_reload = false;
        while let Ok(evt) = self.rx.try_recv() {
            use notify::DebouncedEvent::*;
            match evt {
                NoticeWrite(ref path) |
                Write(ref path) |
                Create(ref path) => {
                    if *path == self.path {
                        should_reload = true;
                    }
                }
                _ => {}
            }
        }

        if should_reload || self.sym.is_none() {
            self.reload_now()
        } else {
            Ok(())
        }
    }

    /// Immediately reload the library without checking whether it has changed.
    ///
    /// This first calls `unload` on the currently loaded library, then unloads
    /// the dynamic library. Next, it loads the new dynamic library, and calls
    /// `reload` on that. If the new library fails to load, this method will
    /// return an `Err` and the `Reloadable` will be left with no library
    /// loaded.
    ///
    /// [`update`]: struct.Reloadable.html#method.update
    pub fn reload_now(&mut self) -> Result<(), Error> {
        if let Some(AppSym { ref mut api, .. }) = self.sym {
            (unsafe { &***api }.unload)(&mut self.host, Self::get_state_ptr(&mut self.state));
        }
        self.sym = None;
        let sym = AppSym::new(&self.path)?;
        // @Avoid reallocating if unnecessary
        self.realloc_buffer((unsafe { &**sym.api }.size)());
        (unsafe { &**sym.api }.reload)(&mut self.host, Self::get_state_ptr(&mut self.state));
        self.sym = Some(sym);

        Ok(())
    }

    /// Call the update method on the library.
    ///
    /// If no library is currently loaded, this does nothing and returns
    /// [`ShouldQuit::No`](enum.ShouldQuit.html#).
    pub fn update(&mut self) -> ShouldQuit {
        if let Some(AppSym { ref mut api, .. }) = self.sym {
            (unsafe { &***api }.update)(&mut self.host, Self::get_state_ptr(&mut self.state))
        } else {
            ShouldQuit::No
        }
    }

    /// Reallocate the buffer used to store the `State`.
    fn realloc_buffer(&mut self, size: usize) {
        let alloc_size_u64s = (size + 7) / 8;
        self.state.resize(alloc_size_u64s, 0);
    }

    /// Get a void pointer to the `State` buffer.
    fn get_state_ptr(buffer: &mut Vec<u64>) -> *mut () {
        buffer.as_mut_ptr() as *mut ()
    }

    /// Get a reference to the `Host` struct>
    pub fn host(&self) -> &Host { &self.host }

    /// Get a mutable reference to the `Host` struct.
    pub fn host_mut(&mut self) -> &mut Host { &mut self.host }

    /// Save a copy of the state
    pub fn save_state(&self) -> SaveState {
        SaveState { state: self.state.clone() }
    }

    /// Load a copy of the state
    pub fn load_state(&mut self, state: &SaveState) {
        self.state.clear();
        self.state.extend_from_slice(state.state.as_slice());
    }
}

/// A saved copy of the state
pub struct SaveState {
    state: Vec<u64>,
}

impl<Host> Drop for Reloadable<Host> {
    fn drop(&mut self) {
        if let Some(AppSym { ref mut api, .. }) = self.sym {
            unsafe {
                ((***api).deinit)(&mut self.host, Self::get_state_ptr(&mut self.state));
            }
        }
    }
}

/// Exported for compilation reasons but not useful, only look if you're curious.
///
/// This module holds to the `ReloadApi` struct, which is what what is looked up
/// by the `Reloadable` in order to communicate with the reloadable library. It
/// needs to be exported in order to avoid forcing the type definition into the
/// pub symbols of the wrapped library. An instance of `ReloadApi` called
/// `RELOAD_API` is generated by the [`live_reload!`][] macro.
///
/// [`live_reload!`]: ../macro.live_reload.html
pub mod internals {
    /// Contains function pointers for all the parts of the reloadable object lifecycle.
    #[repr(C)]
    pub struct ReloadApi<Host> {
        /// Returns the size of the State struct so that the host can allocate
        /// space for it.
        pub size: fn() -> usize,
        /// Initializes the State struct when the program is first started.
        pub init: fn(&mut Host, *mut ()),
        /// Makes any necessary updates when the program is reloaded.
        ///
        /// This will probably be normally empty. If you changed the State
        /// struct since the last compile, then it won't necessarily be
        /// correctly initialized. For safety, you should make your State struct
        /// `#[repr(C)]` and only add members at the end.
        pub reload: fn(&mut Host, *mut ()),
        /// Update the
        pub update: fn(&mut Host, *mut ()) -> super::ShouldQuit,
        /// Prepare for the library to be unloaded before a new version loads.
        ///
        /// This will probably normally be empty except for short periods in
        /// development when you're making lots of live changes and need to do
        /// some kind of migration.
        pub unload: fn(&mut Host, *mut ()),
        /// Do final shutdowns before the program completely quits.
        pub deinit: fn(&mut Host, *mut ()),
    }
}
