//! Answering IPC requests (see `io::ipc` for the socket and protocol).

use super::*;
use crate::io::ipc::{error_reply, ok_reply};

impl App {
    /// Run one request line from the socket and return its JSON reply.
    pub(super) fn on_ipc(&mut self, event_loop: &ActiveEventLoop, request: &str) -> String {
        let request = request.trim();
        if let Some(what) = request.strip_prefix("get ") {
            return match what.trim() {
                "current" => {
                    let info = self.script.shared.borrow().info.clone();
                    ok_reply(Some(
                        info.map(|i| i.to_json()).unwrap_or(serde_json::Value::Null),
                    ))
                }
                other => error_reply(&format!("get: unknown query '{other}' (try: current)")),
            };
        }
        match request.parse::<Command>() {
            Ok(cmd) => {
                self.exec(event_loop, cmd);
                // Anything the command queued (Lua hooks, keybind functions).
                self.drain_queue(event_loop);
                ok_reply(None)
            }
            Err(e) => error_reply(&e),
        }
    }
}
