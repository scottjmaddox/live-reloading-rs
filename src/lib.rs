// Support using part of the library without the standard library!
#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! A library for doing live-reloading game development.
//!
//! This is inspired by the article ["Interactive Programming in C"][] by Chris
//! Wellons, and the video ["Loading Game Code Dynamically"][] from Handmade
//! Hero by Casey Muratori.
//!
//! ["Interactive Programming in C"]: http://nullprogram.com/blog/2014/12/23/
//! ["Loading Game Code Dynamically"]: https://www.youtube.com/watch?v=WMSBRk5WG58
//!
//! The general idea is that your main host program is a wrapper around a
//! dynamic library that does all the interesting work of your game. This means
//! that you can simply reload the library while the game is still running, and
//! have your game update live. As a consequence however, you can't have any
//! global state in your library, everything must be owned by the host in order
//! to avoid getting unloaded with the library.
//!
//! In order to call back into the host program, you specify a `Host` type
//! containing function pointers to services in the host program. This `Host`
//! struct will be passed along with your program state. The `Host` type should
//! always be defined in a separate module so that both the host program and the
//! reloadable library use a consistent host type. The current recommended
//! method is to define your host type in its own module. Then use that module
//! from both the host and the reloadable library. If your project organization
//! puts the common module in a parent, you can always use the `#[path=...]`
//! meta on the module, for example:
//!
//! ```rust,ignore
//! #[path="../host_api.rs"]
//! mod host_api;
//! ```
//!
//! While designing your host and library, keep in mind the role of the two
//! communication types:
//!
//! - State is isolated to the reloadable library, the main program knows
//!   nothing about it except for its size so that it can keep it allocated.
//!   This lets you change the contents of the State struct without restarting
//!   the whole program. This is intended to handle the state of the game world,
//!   independent of the host program.
//!
//! - Host is defined by the host program and the layout is visible to both
//!   sides of the bridge. This means that it has to remain the same during a
//!   run of the game engine. This should hold resources that can only be
//!   produced by the host program, and function pointers to services that can
//!   only be provided by the host program. (Anything that requires global state
//!   like system allocators, graphics contexts, input handling, etc etc.)
//!
//! See the Host Example and Library Example sections for instructions on how to
//! build a reloadable application.
//!
//! # Host Example
//!
//! A program that hosts a reloadable library will need to load the library, and
//! then periodically reload it. The [`Reloadable`][] automatically installs a
//! filesystem watcher for you so that it knows when the library file has been
//! updated or replaced, and the [`reload`][] method will only actually perform
//! a reload if the file has changed. The core of your main loop will therefore
//! usually look something like this:
//!
//! ```rust,no_run
//! use std::thread;
//!
//! mod host_api {
//!     // This should always be in a different file
//!     pub struct HostApi {
//!         pub print: fn(&str),
//!     }
//! }
//! use host_api::HostApi;
//!
//! type App = live_reload::Reloadable<HostApi>;
//!
//! fn print(msg: &str) {
//!     print!("{}", msg);
//! }
//!
//! fn main() {
//!     let mut prog = App::new(
//!         "target/debug/libreload.dylib",
//!         HostApi { print: print },
//!     ).expect("Should successfully load");
//!     'main: loop {
//!         if prog.update() == live_reload::ShouldQuit::Yes {
//!             break 'main;
//!         }
//!         prog.reload().expect("Should successfully reload");
//!     }
//! }
//! ```
//!
//! # Library Example
//!
//! A live-reloadable library needs to register its entry-points so that the
//! host program can find them. The [`live_reload!`][] macro lets you do this
//! conveniently.
//!
//! The lifecycle of your reloadable library will happen in a few stages:
//!
//! - `init` gets called at the very beginning of the program, when the host
//!   starts for the first time.
//! - `reload` gets called on each library load, including the first time. This
//!   should be usually empty, but when you're in development, you might want to
//!   reset things here, or migrate data, or things like that. The pointer
//!   you're passed will refer to the same struct that you had when the previous
//!   library was unloaded, so it might not be properly initialized. You should
//!   try to make your struct be `#[repr(C)]`, and only add members at the end
//!   to minimize the problems of reloading.
//! - `update` gets called at the host program's discretion. You'll probably end
//!   up calling this once per frame. In addition to doing whatever work you
//!   were interested in, `update` also returns a value indicating whether the
//!   host program should quit.
//! - `unload` gets called before a library unloads. This will probably be empty
//!   even more often than `reload`, but you might need it for some debugging or
//!   data migration purpose.
//! - `deinit` gets called when the host program is actually shutting down--it's
//!   called on the drop of the [`Reloadable`][].
//!
//! Here's an example of a live-reloadable library that handles a counter.
//!
//! ```rust
//! #[macro_use] extern crate live_reload;
//! # fn main() {}
//! use live_reload::ShouldQuit;
//!
//! mod host_api {
//!     // This should always be in a different file.
//!     pub struct Host {
//!         pub print: fn(&str),
//!     }
//! }
//!
//! use host_api::Host;
//!
//! live_reload! {
//!     host: Host;
//!     state: State;
//!     init: my_init;
//!     reload: my_reload;
//!     update: my_update;
//!     unload: my_unload;
//!     deinit: my_deinit;
//! }
//!
//! struct State {
//!     counter: u64,
//! }
//!
//! fn my_init(host: &mut Host, state: &mut State) {
//!     state.counter = 0;
//!     (host.print)("Init! Counter: 0.");
//! }
//!
//! fn my_reload(host: &mut Host, state: &mut State) {
//!     (host.print)(&format!("Reloaded at {}.", state.counter));
//! }
//!
//! fn my_update(host: &mut Host, state: &mut State) -> ShouldQuit {
//!     state.counter += 1;
//!     (host.print)(&format!("Counter: {}.", state.counter));
//!     ShouldQuit::No
//! }
//!
//! fn my_unload(host: &mut Host, state: &mut State) {
//!     (host.print)(&format!("Unloaded at {}.", state.counter));
//! }
//!
//! fn my_deinit(host: &mut Host, state: &mut State) {
//!     (host.print)(&format!("Goodbye! Reached a final value of {}.", state.counter));
//! }
//! ```
//! 
//! # State Saving and Loading
//! 
//! Since live reloading pairs well with state saving and loading, [`Reloadable`][]
//! provides the [`save_state`][] and [`load_state`][] methods. The [`save_state`][]
//! method returns a [`SaveState`][] struct, which contains a copy of the state at the
//! time that [`save_state`][] was called, while the [`load_state`][] method accepts
//! a reference to a [`SaveState`][] struct, and loads the saved state.
//!
//! [`Reloadable`]: struct.Reloadable.html
//! [`reload`]: struct.Reloadable.html#method.reload
//! [`save_state`]: struct.Reloadable.html#method.save_state
//! [`load_state`]: struct.Reloadable.html#method.load_state
//! [`live_reload!`]: macro.live_reload.html
//! 
//! # Support for `no_std` Libraries
//! 
//! If you want your library to be `no_std`, then you can import `live-reload`
//! in `no_std` mode by changing the `live-reload` dependency in your `Cargo.toml`
//! to this:
//! 
//! ```toml
//! [dependencies.live-reload]
//! path = "../.."
//! default-features = false
//! features = []
//! ```

#[cfg(feature = "std")]
extern crate notify;
#[cfg(feature = "std")]
extern crate libloading;

#[cfg(feature = "std")]
mod with_std;
#[cfg(feature = "std")]
pub use with_std::*;

/// Should the main program quit? More self-documenting than a boolean!
///
/// This type is returned by the [`update`][] method, since with a boolean it's
/// often unclear if `true` means "should continue" or "should quit".
///
/// [`update`]: struct.Reloadable.html#method.update
#[derive(Debug, PartialEq, Eq)]
pub enum ShouldQuit {
    /// The wrapped library thinks the main program should continue running.
    No = 0,
    /// The wrapped library thinks the main program should quit now.
    Yes = 1,
}

/// Declare the API functions for a live-reloadable library.
///
/// This generates wrappers around higher-level lifecycle functions, and then
/// exports them in a struct that the reloader can find.
///
/// You need to to specify the host API type, define a struct that represents
/// the state of your program, and then define methods for `init`, `reload`,
/// `update`, `unload`, and `deinit`. `init` and `deinit` are called at the very
/// beginning and end of the program, and `reload` and `unload` are called
/// immediately after and before the library is loaded/reloaded. `update` is
/// called by the wrapping application as needed.
///
/// # Example
///
/// ```rust
/// # #[macro_use] extern crate live_reload;
/// # fn main() {}
/// # #[repr(C)] struct State {}
/// # mod host_api { pub struct Host; }
/// # use host_api::Host;
/// # fn my_init(_: &mut Host, _: &mut State) {}
/// # fn my_reload(_: &mut Host, _: &mut State) {}
/// # fn my_unload(_: &mut Host, _: &mut State) {}
/// # fn my_deinit(_: &mut Host, _: &mut State) {}
/// # use live_reload::ShouldQuit;
/// # fn my_update(_: &mut Host, _: &mut State) -> ShouldQuit { ShouldQuit::No }
/// live_reload! {
///     host: host_api::Host;
///     state: State;
///     init: my_init;
///     reload: my_reload;
///     update: my_update;
///     unload: my_unload;
///     deinit: my_deinit;
/// }
/// ```
#[macro_export]
macro_rules! live_reload {
    (host: $Host:ty;
     state: $State:ty;
     init: $init:ident;
     reload: $reload:ident;
     update: $update:ident;
     unload: $unload:ident;
     deinit: $deinit:ident;) => {

        fn cast<'a>(raw_state: *mut ()) -> &'a mut $State {
            unsafe { &mut *(raw_state as *mut $State) }
        }

        fn init_wrapper(host: &mut $Host, raw_state: *mut ()) {
            $init(host, cast(raw_state))
        }

        fn reload_wrapper(host: &mut $Host, raw_state: *mut ()) {
            $reload(host, cast(raw_state))
        }

        fn update_wrapper(host: &mut $Host, raw_state: *mut ())
            -> ::live_reload::ShouldQuit
        {
            $update(host, cast(raw_state))
        }

        fn unload_wrapper(host: &mut $Host, raw_state: *mut ()) {
            $unload(host, cast(raw_state))
        }

        fn deinit_wrapper(host: &mut $Host, raw_state: *mut ()) {
            $deinit(host, cast(raw_state))
        }

        #[cfg(feature = "std")]
        use ::std::mem;
        #[cfg(not(feature = "std"))]
        use ::core::mem;

        #[no_mangle]
        pub static RELOAD_API: ::live_reload::internals::ReloadApi<$Host> =
            ::live_reload::internals::ReloadApi
        {
            size: mem::size_of::<$State>,
            init: init_wrapper,
            reload: reload_wrapper,
            update: update_wrapper,
            unload: unload_wrapper,
            deinit: deinit_wrapper,
        };
    }
}
