//! Desktop notifications for failures nobody would otherwise see: when wlens
//! is started from a file manager or launcher there is no terminal for the log.

use std::io::IsTerminal;
use std::process::{Command, Stdio};

/// Tell the user wlens couldn't start. Logs always; also sends a desktop
/// notification (via `notify-send`) when stderr isn't a terminal.
pub fn startup_failure(what: &str, err: &dyn std::fmt::Display) {
    let detail = format!("{err:#}");
    log::error!("{what}: {detail}");
    if std::io::stderr().is_terminal() {
        return;
    }
    let body = match hint(&detail) {
        Some(h) => format!("{what}: {detail}\n\n{h}"),
        None => format!("{what}: {detail}"),
    };
    // Fire and forget: notify-send outlives us if we exit first, and a
    // missing binary just means no notification.
    let sent = Command::new("notify-send")
        .args([
            "--app-name=wlens",
            "--urgency=critical",
            "--icon=dialog-error",
        ])
        .arg("wlens couldn't start")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = sent {
        log::warn!("notify-send: {e}");
    }
}

/// A plain-language explanation for errors whose cause lies outside wlens.
fn hint(detail: &str) -> Option<&'static str> {
    let d = detail.to_lowercase();
    if d.contains("memory") {
        Some(
            "The GPU is out of memory: another program (a loaded AI model, a game…) is probably using it. Free some VRAM and try again.",
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_memory_gets_a_hint() {
        assert!(hint("request device: Not enough memory left").is_some());
        assert!(hint("surface unsupported by adapter").is_none());
    }
}
