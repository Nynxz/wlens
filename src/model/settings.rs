//! Behavioural options plus the theme.
//!
//! Every option is addressable by a string key so the same code path serves
//! Lua (`wlens.opt.fit = "cover"`), keybinds (`set fit cover`) and IPC.
//! Dotted keys (`image.radius`) are theme properties, see `theme.rs`.

pub use crate::model::theme::Value;
use crate::model::theme::{Theme, parse_bool, parse_num};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    /// 1 image pixel = 1 physical screen pixel.
    None,
    /// Like `Contain`, but never scale small images up.
    Shrink,
    /// Scale to fit entirely inside the window.
    Contain,
    /// Scale to fill the window, cropping overflow.
    Cover,
    Width,
    Height,
}

impl FitMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "none" | "actual" => Self::None,
            "shrink" => Self::Shrink,
            "contain" | "full" | "fit" => Self::Contain,
            "cover" | "crop" | "fill" => Self::Cover,
            "width" => Self::Width,
            "height" => Self::Height,
            _ => {
                return Err(format!(
                    "unknown fit mode '{s}' (none|shrink|contain|cover|width|height)"
                ));
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Shrink => "shrink",
            Self::Contain => "contain",
            Self::Cover => "cover",
            Self::Width => "width",
            Self::Height => "height",
        }
    }
}

/// How far the image may be dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanMode {
    /// Smaller-than-window images stay centred.
    Center,
    /// Small images move freely but stay inside the window; large images
    /// can't leave a gap at their edges.
    #[default]
    Bounded,
    /// Anywhere, as long as a sliver stays on screen.
    Free,
}

impl PanMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "center" | "centre" => Self::Center,
            "bounded" => Self::Bounded,
            "free" => Self::Free,
            _ => return Err(format!("unknown pan mode '{s}' (center|bounded|free)")),
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Bounded => "bounded",
            Self::Free => "free",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    Linear,
    Nearest,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub fit: FitMode,
    pub filter: Filter,
    /// Multiplicative step for `zoom in` / `zoom out`.
    pub zoom_step: f64,
    /// Wrap around at the ends of the file list.
    pub wrap: bool,
    pub flat: bool,
    /// How many images to decode ahead/behind the current one.
    pub preload: usize,
    /// Decoded-image cache budget in MiB.
    pub cache_mb: usize,
    /// Size limit of the download cache (~/.cache/wlens/remote), MiB;
    /// least recently used files go first, at startup.
    pub remote_cache_mb: usize,
    pub pan: PanMode,
    /// When a video ends: "loop", "next" (go on to the next item) or "stop".
    pub video_end: String,
    /// HTTP / SOCKS proxy for everything wlens fetches ("" = none / *_PROXY env).
    pub proxy: String,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fit: FitMode::Shrink,
            filter: Filter::Linear,
            zoom_step: 1.25,
            wrap: false,
            flat: false,
            preload: 2,
            cache_mb: 512,
            remote_cache_mb: 2048,
            pan: PanMode::default(),
            video_end: "loop".into(),
            proxy: String::new(),
            theme: Theme::default(),
        }
    }
}

#[cfg(test)]
pub const KEYS: &[&str] = &[
    "fit",
    "filter",
    "zoom_step",
    "wrap",
    "gallery.flat",
    "preload",
    "cache_mb",
    "remote_cache_mb",
    "pan",
    "video_end",
    "proxy",
];

impl Settings {
    /// Set an option from its string form. `toggle` flips booleans.
    pub fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        match key {
            "fit" => self.fit = FitMode::parse(v)?,
            "filter" => {
                self.filter = match v {
                    "linear" => Filter::Linear,
                    "nearest" => Filter::Nearest,
                    "toggle" => match self.filter {
                        Filter::Linear => Filter::Nearest,
                        Filter::Nearest => Filter::Linear,
                    },
                    _ => return Err(format!("unknown filter '{v}' (linear|nearest)")),
                }
            }
            "zoom_step" => {
                let s: f64 = parse_num(v)?;
                if s <= 1.0 {
                    return Err("zoom_step must be > 1".into());
                }
                self.zoom_step = s;
            }
            "gallery.flat" => self.flat = parse_bool(v, self.flat)?,
            "wrap" => self.wrap = parse_bool(v, self.wrap)?,
            "preload" => self.preload = parse_num(v)?,
            "cache_mb" => self.cache_mb = parse_num(v)?,
            "remote_cache_mb" => self.remote_cache_mb = parse_num(v)?,
            "pan" => self.pan = PanMode::parse(v)?,
            "proxy" => {
                crate::io::remote::set_proxy(v)?;
                self.proxy = v.trim().to_string();
            }
            "video_end" => {
                if !matches!(v, "loop" | "next" | "stop") {
                    return Err(format!("video_end: '{v}' (loop|next|stop)"));
                }
                self.video_end = v.into();
            }
            _ if key.contains('.') => return self.theme.set(key, v),
            _ => return Err(format!("unknown option '{key}'")),
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        use Value::*;
        Some(match key {
            "fit" => Str(self.fit.name().into()),
            "filter" => Str(match self.filter {
                Filter::Linear => "linear".into(),
                Filter::Nearest => "nearest".into(),
            }),
            "zoom_step" => Num(self.zoom_step),
            "gallery.flat" => Bool(self.flat),
            "wrap" => Bool(self.wrap),
            "preload" => Num(self.preload as f64),
            "cache_mb" => Num(self.cache_mb as f64),
            "remote_cache_mb" => Num(self.remote_cache_mb as f64),
            "pan" => Str(self.pan.name().into()),
            "video_end" => Str(self.video_end.clone()),
            "proxy" => Str(self.proxy.clone()),
            _ => return self.theme.get(key),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_roundtrip() {
        let mut s = Settings::default();
        for key in KEYS {
            let v = match s.get(key).unwrap() {
                Value::Str(s) => s,
                Value::Num(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
            };
            s.set(key, &v).unwrap_or_else(|e| panic!("{key}: {e}"));
        }
        s.set("wrap", "toggle").unwrap();
        assert!(s.wrap);
        assert!(s.set("nope", "1").is_err());
        s.set("image.radius", "12").unwrap();
        assert_eq!(s.theme.image_radius, 12.0);
    }
}
