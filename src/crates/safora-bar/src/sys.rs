//! System helpers for the bar: everything lives in safora-common; only the IPC thread is bar-specific.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

pub use safora_common::sys::*;
use safora_common::ipc::{socket_path, Request, State};

// ------------------------------------------------------------------ compositor

/// Subscribe to compositor state forever, reconnecting if the compositor restarts.
pub fn ipc_thread(tx: async_channel::Sender<State>) {
    loop {
        if let Ok(mut stream) = UnixStream::connect(socket_path()) {
            let mut line = serde_json::to_string(&Request::Subscribe).unwrap_or_default();
            line.push('\n');
            if stream.write_all(line.as_bytes()).is_ok() {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Ok(state) = serde_json::from_str::<State>(&line) {
                        if tx.send_blocking(state).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

