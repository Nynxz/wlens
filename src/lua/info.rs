//! The current item as scripts and UI text see it.

use super::*;

/// Snapshot of the current item, exposed as `wlens.current()` and to UI text.
#[derive(Clone, Default)]
pub struct ImageInfo {
    /// Path or URL.
    pub path: String,
    /// Label if the item has one, else the file name.
    pub name: String,
    pub dir: String,
    pub remote: bool,
    pub index: usize,
    pub count: usize,
    pub width: u32,
    pub height: u32,
    pub zoom: f64,
    /// The view is fitted (not manually zoomed/panned).
    pub fitted: bool,
    /// The image is bigger than the window on some axis (so drags pan it).
    pub zoomed_in: bool,
    pub has_prev: bool,
    pub has_next: bool,
    /// "image" or "gallery".
    pub mode: &'static str,
    pub container: bool,
    pub item_count: Option<String>,
    pub meta: Option<mlua::Table>,
    /// The current item's picture isn't on screen yet (downloading, decoding).
    pub loading: bool,
    pub depth: usize,
    /// Name of the open archive or directory, or the title supplied by a script.
    pub level: Option<String>,
    /// Animated images: frame count (0 for stills) and current frame (1-based).
    pub frames: usize,
    pub frame: usize,
    /// Video playback (when the item is a video).
    pub video: bool,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub file_size: u64,
}

impl ImageInfo {
    /// The same fields as `wlens.current()`, as JSON (for IPC).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "name": self.name,
            "dir": self.dir,
            "remote": self.remote,
            "index": self.index + 1,
            "count": self.count,
            "width": self.width,
            "height": self.height,
            "zoom": self.zoom,
            "fitted": self.fitted,
            "zoomed_in": self.zoomed_in,
            "has_prev": self.has_prev,
            "has_next": self.has_next,
            "mode": self.mode,
            "container": self.container,
            "count_in": self.item_count,
            "meta": self.meta.as_ref().and_then(|m| serde_json::to_value(m).ok()),
            "loading": self.loading,
            "depth": self.depth,
            "level": self.level,
            "frames": self.frames,
            "frame": self.frame,
            "video": self.video,
            "position": self.position,
            "duration": self.duration,
            "paused": self.paused,
            "volume": self.volume,
            "muted": self.muted,
            "size": self.file_size,
        })
    }

    /// Template field lookup for `{key}` placeholders in UI text.
    pub fn field(&self, key: &str, settings: &Settings) -> Option<String> {
        if let Some(k) = key.strip_prefix("meta.") {
            // `{meta.title}`: strings and numbers; missing -> empty.
            let v: mlua::Value = self.meta.as_ref()?.get(k).ok()?;
            return Some(match v {
                mlua::Value::String(s) => s.to_string_lossy(),
                mlua::Value::Integer(n) => n.to_string(),
                mlua::Value::Number(n) => n.to_string(),
                mlua::Value::Boolean(b) => b.to_string(),
                _ => String::new(),
            });
        }
        Some(match key {
            "name" => self.name.clone(),
            "path" => self.path.clone(),
            "dir" => self.dir.clone(),
            "index" => (self.index + 1).to_string(),
            "count" => self.count.to_string(),
            "width" => self.width.to_string(),
            "height" => self.height.to_string(),
            "zoom" => format!("{:.0}", self.zoom * 100.0),
            "fitted" => self.fitted.to_string(),
            "mode" => self.mode.to_string(),
            "position" => clock(self.position),
            "progress" => format!(
                "{:.4}",
                if self.duration > 0.0 {
                    self.position / self.duration
                } else {
                    0.0
                }
            ),
            "volume" => format!("{:.0}", self.volume * 100.0),
            "duration" => clock(self.duration),
            "level" => self.level.clone().unwrap_or_default(),
            "depth" => self.depth.to_string(),
            "size" => {
                if self.file_size > 0 {
                    human_size(self.file_size)
                } else {
                    String::new()
                }
            }
            "fit" => settings.fit.name().to_string(),
            "filter" => match settings.filter {
                crate::model::settings::Filter::Linear => "linear".into(),
                crate::model::settings::Filter::Nearest => "nearest".into(),
            },
            _ => return None,
        })
    }
}

/// Seconds -> "m:ss" (or "h:mm:ss").
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn human_size(b: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}
