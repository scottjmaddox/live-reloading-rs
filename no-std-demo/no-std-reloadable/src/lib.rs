#![no_std]

#[macro_use]
extern crate live_reload;

#[path="../../src/shared_api.rs"]
mod shared_api;

use live_reload::ShouldQuit;
use shared_api::Host;

live_reload! {
    host: Host;
    state: State;
    init: init;
    reload: reload;
    update: update;
    unload: unload;
    deinit: deinit;
}

#[repr(C)]
struct State {
    counter: usize,
}

fn init(host: &mut Host, state: &mut State) {
    state.counter = 0;
    (host.print)("Init! Counter: 0.\n");
}

fn reload(host: &mut Host, state: &mut State) {
    let mut buf = Buffer::new();
    write!(&mut buf, "Reloaded at {}.\n",
        state.counter).unwrap();
    (host.print)(buf.as_str());
}

fn update(host: &mut Host, state: &mut State) -> ShouldQuit {
    state.counter += 2;
    let mut buf = Buffer::new();
    write!(&mut buf, "Counter: {}.\n",
        state.counter).unwrap();
    (host.print)(buf.as_str());
    ShouldQuit::No
}

fn unload(host: &mut Host, state: &mut State) {
    let mut buf = Buffer::new();
    write!(&mut buf, "Unloaded at {}.\n",
        state.counter).unwrap();
    (host.print)(buf.as_str());
}

fn deinit(host: &mut Host, state: &mut State) {
    let mut buf = Buffer::new();
    write!(&mut buf, "Goodbye! Reached a final value of {}.\n",
        state.counter).unwrap();
    (host.print)(buf.as_str());
}

const BUFFER_CAPACITY: usize = 1024;

struct Buffer {
    b: [u8; BUFFER_CAPACITY],
    i: usize,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            b: [0; BUFFER_CAPACITY],
            i: 0,
        }
    }

    pub fn unused(&self) -> usize {
        BUFFER_CAPACITY - self.i
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.i]).unwrap()
    }
}

use core::fmt::Write;
use core::fmt::Error;
impl Write for Buffer {
    fn write_str(&mut self, s: &str) -> Result<(), Error> {
        let bytes = s.as_bytes();
        if bytes.len() > self.unused() {
            return Err(Error {});
        }
        for byte in bytes {
            self.b[self.i] = *byte;
            self.i += 1;
        }
        Ok(())
    }
}
