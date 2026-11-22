//! Control a running viewer over a Unix socket.
//!
//! Each viewer listens on `$XDG_RUNTIME_DIR/wlens/<pid>.sock` (also exported
//! to children as `$WLENS_SOCKET`). The protocol is line based: each request
//! line is a command (the same language as keybinds, e.g. `next`, `zoom in`)
//! or a query (`get current`), and each gets one JSON line back:
//! `{"ok":true}`, `{"ok":true,"data":{…}}` or `{"ok":false,"error":"…"}`.
//!
//! Requests are handed to the main thread as `UserEvent::Ipc` and answered
//! from there, so they run exactly like a keypress would.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Longest request line accepted (a `lua …` command can be long; not huge).
const MAX_LINE: usize = 64 * 1024;

/// Where sockets live: `$XDG_RUNTIME_DIR/wlens`.
fn socket_dir() -> Option<PathBuf> {
    dirs::runtime_dir().map(|d| d.join("wlens"))
}

/// A bound socket, removed again when dropped.
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

impl Server {
    /// Bind this process's socket. `None` (logged) when there's no runtime
    /// dir or binding fails: the viewer works without IPC.
    pub fn bind() -> Option<Server> {
        let bind = || -> anyhow::Result<Server> {
            let dir = socket_dir().context("no $XDG_RUNTIME_DIR")?;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .with_context(|| format!("create {}", dir.display()))?;
            let path = dir.join(format!("{}.sock", std::process::id()));
            // A previous process with our pid can't still be running.
            let _ = std::fs::remove_file(&path);
            let listener =
                UnixListener::bind(&path).with_context(|| format!("bind {}", path.display()))?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Server { listener, path })
        };
        match bind() {
            Ok(s) => {
                log::debug!("ipc: listening on {}", s.path.display());
                Some(s)
            }
            Err(e) => {
                log::warn!("ipc disabled: {e:#}");
                None
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept connections in the background, forwarding requests to the
    /// event loop. The socket file stays until the returned guard drops.
    pub fn serve(self, proxy: EventLoopProxy<UserEvent>) -> SocketGuard {
        let guard = SocketGuard(self.path.clone());
        let listener = self.listener;
        let spawned = std::thread::Builder::new()
            .name("wlens-ipc".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    let Ok(conn) = conn else { continue };
                    let proxy = proxy.clone();
                    let _ = std::thread::Builder::new()
                        .name("wlens-ipc-conn".into())
                        .spawn(move || handle(conn, proxy));
                }
            });
        if let Err(e) = spawned {
            log::warn!("ipc disabled: {e}");
        }
        guard
    }
}

/// Removes the socket file on drop.
pub struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// One client: answer each request line in turn until it hangs up.
fn handle(conn: UnixStream, proxy: EventLoopProxy<UserEvent>) {
    let Ok(mut out) = conn.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    loop {
        line.clear();
        match (&mut reader).take(MAX_LINE as u64 + 1).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let reply = if line.len() > MAX_LINE {
            error_reply("request too long")
        } else if line.trim().is_empty() {
            continue;
        } else {
            let (tx, rx) = mpsc::channel();
            let request = line.trim().to_owned();
            if proxy
                .send_event(UserEvent::Ipc { request, reply: tx })
                .is_err()
            {
                return; // the viewer is shutting down
            }
            rx.recv().unwrap_or_else(|_| error_reply("viewer closed"))
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

pub fn ok_reply(data: Option<serde_json::Value>) -> String {
    match data {
        Some(d) => serde_json::json!({ "ok": true, "data": d }).to_string(),
        None => serde_json::json!({ "ok": true }).to_string(),
    }
}

pub fn error_reply(msg: &str) -> String {
    serde_json::json!({ "ok": false, "error": msg }).to_string()
}

/// The socket a client should talk to: `explicit`, else `$WLENS_SOCKET`,
/// else the most recently started viewer that's still running.
pub fn find_socket(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_owned());
    }
    if let Some(p) = std::env::var_os("WLENS_SOCKET").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let dir = socket_dir().context("no $XDG_RUNTIME_DIR")?;
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    socks.sort_by_key(|s| std::cmp::Reverse(s.0)); // newest first
    for (_, path) in socks {
        match UnixStream::connect(&path) {
            Ok(_) => return Ok(path),
            // Left behind by a viewer that crashed.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                let _ = std::fs::remove_file(&path);
            }
            Err(_) => {}
        }
    }
    bail!("no running wlens found")
}

/// Send one request and return the reply's `data` (or `None` for a plain ok).
pub fn send(socket: &Path, request: &str) -> anyhow::Result<Option<serde_json::Value>> {
    if request.contains('\n') {
        bail!("a request is a single line");
    }
    let mut conn =
        UnixStream::connect(socket).with_context(|| format!("connect to {}", socket.display()))?;
    conn.set_read_timeout(Some(Duration::from_secs(10)))?;
    writeln!(conn, "{request}")?;
    let mut reply = String::new();
    BufReader::new(conn).read_line(&mut reply)?;
    let v: serde_json::Value =
        serde_json::from_str(&reply).with_context(|| format!("bad reply: {reply:?}"))?;
    if v["ok"].as_bool() == Some(true) {
        Ok(v.get("data").cloned())
    } else {
        Err(anyhow!(
            "{}",
            v["error"].as_str().unwrap_or("request failed")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_one_json_line() {
        assert_eq!(ok_reply(None), r#"{"ok":true}"#);
        let r = error_reply("next: bad count 'x'\nsecond line");
        assert!(!r.contains('\n'), "newlines are escaped");
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn send_round_trips_through_a_socket() {
        let dir = std::env::temp_dir().join(format!("wlens-ipc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut out = conn.try_clone().unwrap();
            let mut line = String::new();
            BufReader::new(conn).read_line(&mut line).unwrap();
            let reply = if line.trim() == "get current" {
                ok_reply(Some(serde_json::json!({ "index": 3 })))
            } else {
                error_reply("nope")
            };
            writeln!(out, "{reply}").unwrap();
        });
        let data = send(&path, "get current").unwrap().unwrap();
        assert_eq!(data["index"], 3);
        server.join().unwrap();
        assert!(send(&path, "two\nlines").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
