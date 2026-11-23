//! Visual theme: everything about how wlens *looks* (as opposed to
//! `Settings`, which is about how it behaves).
//!
//! Every property has a dotted path (`image.shadow.size`), so Lua tables,
//! `set image.radius 12` keybinds and IPC all share one setter. All lengths
//! are logical pixels; the renderer multiplies by the output scale.

use std::fmt;

use crate::geom::anim::Easing;

/// sRGB colour with straight alpha, components 0..=1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const TRANSPARENT: Self = Self([0.0; 4]);

    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        // Names first: "none" is also four hex-looking characters long.
        if t.eq_ignore_ascii_case("transparent") || t.eq_ignore_ascii_case("none") {
            return Ok(Self([0.0; 4]));
        }
        if let Some(args) = t.strip_prefix("oklch(").and_then(|r| r.strip_suffix(')')) {
            return Self::parse_oklch(args).ok_or_else(|| format!("bad colour '{s}'"));
        }
        if let Some(args) = t
            .strip_prefix("rgba(")
            .or_else(|| t.strip_prefix("rgb("))
            .and_then(|r| r.strip_suffix(')'))
        {
            return Self::parse_rgb(args).ok_or_else(|| format!("bad colour '{s}'"));
        }
        let hex = t.strip_prefix('#').unwrap_or(t);
        let digit = |i: usize, n: usize| {
            u8::from_str_radix(hex.get(i..i + n).unwrap_or("x"), 16)
                .map_err(|_| format!("bad colour '{s}'"))
        };
        let (r, g, b, a) = match hex.len() {
            3 | 4 => {
                let c = |i: usize| digit(i, 1).map(|v| v * 17);
                let a = if hex.len() == 4 { c(3)? } else { 255 };
                (c(0)?, c(1)?, c(2)?, a)
            }
            6 | 8 => {
                let c = |i: usize| digit(i * 2, 2);
                let a = if hex.len() == 8 { c(3)? } else { 255 };
                (c(0)?, c(1)?, c(2)?, a)
            }
            _ if s == "transparent" || s == "none" => (0, 0, 0, 0),
            _ => {
                return Err(format!(
                    "bad colour '{s}' (expected #rgb, #rrggbb or #rrggbbaa)"
                ));
            }
        };
        Ok(Self([r, g, b, a].map(|v| v as f32 / 255.0)))
    }

    /// Split CSS function arguments: "a b c / d" or "a, b, c, d".
    fn css_args(args: &str) -> (Vec<&str>, Option<&str>) {
        let (main, alpha) = match args.split_once('/') {
            Some((m, a)) => (m, Some(a.trim())),
            None => (args, None),
        };
        let parts: Vec<&str> = main.split([' ', ',']).filter(|p| !p.is_empty()).collect();
        (parts, alpha)
    }

    fn css_num(v: &str, percent_of: f32) -> Option<f32> {
        match v.strip_suffix('%') {
            Some(p) => p.parse::<f32>().ok().map(|p| p / 100.0 * percent_of),
            None => v.parse().ok(),
        }
    }

    /// CSS `oklch(L C H [/ A])` -> sRGB (gamut-clipped).
    fn parse_oklch(args: &str) -> Option<Self> {
        let (p, alpha) = Self::css_args(args);
        if p.len() != 3 {
            return None;
        }
        let l = Self::css_num(p[0], 1.0)?;
        let c = Self::css_num(p[1], 0.4)?;
        let h = p[2]
            .trim_end_matches("deg")
            .parse::<f32>()
            .unwrap_or(0.0)
            .to_radians();
        let (a, b) = (c * h.cos(), c * h.sin());
        let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        let lin = [
            4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
            -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
            -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
        ];
        let enc = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            if x <= 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            }
        };
        let alpha = alpha.and_then(|a| Self::css_num(a, 1.0)).unwrap_or(1.0);
        Some(Self([
            enc(lin[0]),
            enc(lin[1]),
            enc(lin[2]),
            alpha.clamp(0.0, 1.0),
        ]))
    }

    /// CSS `rgb(r g b [/ a])` / `rgba(r, g, b, a)`.
    fn parse_rgb(args: &str) -> Option<Self> {
        let (mut p, mut alpha) = Self::css_args(args);
        if p.len() == 4 && alpha.is_none() {
            alpha = p.pop();
        }
        if p.len() != 3 {
            return None;
        }
        let ch = |v: &str| Self::css_num(v, 255.0).map(|n| (n / 255.0).clamp(0.0, 1.0));
        let a = alpha.and_then(|a| Self::css_num(a, 1.0)).unwrap_or(1.0);
        Some(Self([ch(p[0])?, ch(p[1])?, ch(p[2])?, a.clamp(0.0, 1.0)]))
    }

    /// Linear-light RGB (alpha untouched), straight alpha.
    pub fn linear(self) -> [f32; 4] {
        let f = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b, a] = self.0;
        [f(r), f(g), f(b), a]
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let mut out = self.0;
        for (o, b) in out.iter_mut().zip(other.0) {
            *o += (b - *o) * t;
        }
        Self(out)
    }

    pub fn with_alpha(self, a: f32) -> Self {
        let [r, g, b, _] = self.0;
        Self([r, g, b, a])
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.0.map(|c| (c * 255.0).round() as u8);
        write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    None,
    Fade,
    Slide,
    Zoom,
}

impl TransitionKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "none" => Self::None,
            "fade" => Self::Fade,
            "slide" => Self::Slide,
            "zoom" => Self::Zoom,
            _ => return Err(format!("unknown transition '{s}' (none|fade|slide|zoom)")),
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fade => "fade",
            Self::Slide => "slide",
            Self::Zoom => "zoom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Blur radius; 0 disables the shadow.
    pub size: f32,
    pub color: Color,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub window_background: Color,
    /// Ask the compositor for a translucent window (lightbox / blur look).
    /// Only read at startup.
    pub window_transparent: bool,

    /// Space kept around the image when fitting.
    pub image_margin: f32,
    pub image_radius: f32,
    pub image_border: Border,
    pub image_shadow: Shadow,
    pub checker: bool,
    pub checker_a: Color,
    pub checker_b: Color,
    pub checker_size: f32,

    /// Seconds for zoom/pan to glide to the target; 0 = instant.
    pub anim_view: f32,
    pub anim_wheel_zoom: f32,
    /// Seconds for UI fades and hover effects.
    pub anim_ui: f32,
    /// Seconds of mouse inactivity before `show = "hover"` UI hides.
    pub ui_idle: f32,

    pub transition: TransitionKind,
    pub transition_duration: f32,
    pub transition_easing: Easing,

    pub font_family: String,
    pub font_size: f32,
    pub font_color: Color,

    pub tooltip_background: Color,
    pub tooltip_color: Color,
    pub tooltip_radius: f32,
    pub tooltip_size: f32,
    /// Seconds of hovering before a tooltip appears.
    pub tooltip_delay: f32,

    /// Thumbnail cell edge, logical px.
    pub gallery_size: f32,
    pub gallery_gap: f32,
    pub gallery_padding: f32,
    pub gallery_radius: f32,
    /// Crop thumbnails to fill their square (else letterbox them).
    pub gallery_cover: bool,
    /// Cell background / placeholder while a thumbnail loads.
    pub gallery_background: Color,
    pub gallery_selected: Color,
    pub gallery_selected_width: f32,
    pub gallery_hover: Color,
    pub gallery_labels: bool,
    pub gallery_label_color: Color,
    pub gallery_label_size: f32,
    /// Seconds for the image <-> gallery cross-fade and scroll glide.
    pub gallery_anim: f32,
}

impl Default for Theme {
    fn default() -> Self {
        let c = |s| Color::parse(s).unwrap();
        Self {
            window_background: c("#121214"),
            window_transparent: false,
            image_margin: 0.0,
            image_radius: 0.0,
            image_border: Border {
                width: 0.0,
                color: c("#ffffff33"),
            },
            image_shadow: Shadow {
                size: 0.0,
                color: c("#00000099"),
                x: 0.0,
                y: 8.0,
            },
            checker: true,
            checker_a: c("#292929"),
            checker_b: c("#383838"),
            checker_size: 12.0,
            anim_view: 0.0,
            anim_wheel_zoom: 0.16,
            anim_ui: 0.15,
            ui_idle: 1.5,
            transition: TransitionKind::None,
            transition_duration: 0.25,
            transition_easing: Easing::EaseOut,
            font_family: "sans-serif".into(),
            font_size: 14.0,
            font_color: c("#e6e6e6"),
            tooltip_background: c("#101014ee"),
            tooltip_color: c("#f0f0f0"),
            tooltip_radius: 6.0,
            tooltip_size: 12.0,
            tooltip_delay: 0.45,
            gallery_size: 160.0,
            gallery_gap: 10.0,
            gallery_padding: 20.0,
            gallery_radius: 6.0,
            gallery_cover: true,
            gallery_background: c("#ffffff0d"),
            gallery_selected: c("#8aadf4"),
            gallery_selected_width: 2.5,
            gallery_hover: c("#ffffff55"),
            gallery_labels: false,
            gallery_label_color: c("#c4c4ca"),
            gallery_label_size: 11.0,
            gallery_anim: 0.18,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
}

pub fn parse_bool(v: &str, current: bool) -> Result<bool, String> {
    match v {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        "toggle" => Ok(!current),
        _ => Err(format!("expected a boolean, got '{v}'")),
    }
}

pub fn parse_num<T: std::str::FromStr>(v: &str) -> Result<T, String> {
    // "NaN" / "inf" parse as floats but are never a sensible setting.
    let lower = v.trim().to_ascii_lowercase();
    if lower.contains("nan") || lower.contains("inf") {
        return Err(format!("expected a number, got '{v}'"));
    }
    v.trim()
        .parse()
        .map_err(|_| format!("expected a number, got '{v}'"))
}

/// Every settable theme path.
#[cfg(test)]
pub const KEYS: &[&str] = &[
    "window.background",
    "window.transparent",
    "image.margin",
    "image.radius",
    "image.border.width",
    "image.border.color",
    "image.shadow.size",
    "image.shadow.color",
    "image.shadow.x",
    "image.shadow.y",
    "image.checker",
    "image.checker.a",
    "image.checker.b",
    "image.checker.size",
    "anim.view",
    "anim.ui",
    "ui.idle",
    "transition.kind",
    "transition.duration",
    "transition.easing",
    "font.family",
    "font.size",
    "font.color",
    "tooltip.background",
    "tooltip.color",
    "tooltip.radius",
    "tooltip.size",
    "tooltip.delay",
    "gallery.size",
    "gallery.gap",
    "gallery.padding",
    "gallery.radius",
    "gallery.fit",
    "gallery.background",
    "gallery.selected",
    "gallery.selected_width",
    "gallery.hover",
    "gallery.labels",
    "gallery.label_color",
    "gallery.label_size",
    "gallery.anim",
];

impl Theme {
    pub fn set(&mut self, path: &str, v: &str) -> Result<(), String> {
        let len = |v: &str| parse_num::<f32>(v).map(|n| n.max(0.0));
        // Lua arrays flatten to `.1`/`.2`: `checker = {"#111", "#222"}`.
        let path = match path {
            "image.checker.1" => "image.checker.a",
            "image.checker.2" => "image.checker.b",
            "image.checker.enabled" => "image.checker",
            "transition" => "transition.kind",
            "font" => "font.family",
            p => p,
        };
        match path {
            "window.background" => self.window_background = Color::parse(v)?,
            "window.transparent" => {
                self.window_transparent = parse_bool(v, self.window_transparent)?
            }
            "image.margin" => self.image_margin = len(v)?,
            "image.radius" => self.image_radius = len(v)?,
            "image.border.width" => self.image_border.width = len(v)?,
            "image.border.color" => self.image_border.color = Color::parse(v)?,
            "image.shadow.size" => self.image_shadow.size = len(v)?,
            "image.shadow.color" => self.image_shadow.color = Color::parse(v)?,
            "image.shadow.x" => self.image_shadow.x = parse_num(v)?,
            "image.shadow.y" => self.image_shadow.y = parse_num(v)?,
            "image.checker" => self.checker = parse_bool(v, self.checker)?,
            "image.checker.a" => {
                self.checker_a = Color::parse(v)?;
                self.checker = true;
            }
            "image.checker.b" => {
                self.checker_b = Color::parse(v)?;
                self.checker = true;
            }
            "image.checker.size" => self.checker_size = len(v)?.max(1.0),
            "anim.view" => self.anim_view = len(v)?,
            "anim.wheel_zoom" => self.anim_wheel_zoom = len(v)?,
            "anim.ui" => self.anim_ui = len(v)?,
            "ui.idle" => self.ui_idle = len(v)?,
            "transition.kind" => self.transition = TransitionKind::parse(v)?,
            "transition.duration" => self.transition_duration = len(v)?,
            "transition.easing" => self.transition_easing = Easing::parse(v)?,
            "font.family" => self.font_family = v.into(),
            "font.size" => self.font_size = len(v)?.max(1.0),
            "font.color" => self.font_color = Color::parse(v)?,
            "tooltip.background" => self.tooltip_background = Color::parse(v)?,
            "tooltip.color" => self.tooltip_color = Color::parse(v)?,
            "tooltip.radius" => self.tooltip_radius = len(v)?,
            "tooltip.size" => self.tooltip_size = len(v)?.max(1.0),
            "tooltip.delay" => self.tooltip_delay = len(v)?,
            "gallery.size" => self.gallery_size = len(v)?.clamp(32.0, 1024.0),
            "gallery.gap" => self.gallery_gap = len(v)?,
            "gallery.padding" => self.gallery_padding = len(v)?,
            "gallery.radius" => self.gallery_radius = len(v)?,
            "gallery.fit" => {
                self.gallery_cover = match v {
                    "cover" | "crop" => true,
                    "contain" | "fit" => false,
                    "toggle" => !self.gallery_cover,
                    _ => return Err(format!("gallery.fit: expected cover|contain, got '{v}'")),
                }
            }
            "gallery.background" => self.gallery_background = Color::parse(v)?,
            "gallery.selected" => self.gallery_selected = Color::parse(v)?,
            "gallery.selected_width" => self.gallery_selected_width = len(v)?,
            "gallery.hover" => self.gallery_hover = Color::parse(v)?,
            "gallery.labels" => self.gallery_labels = parse_bool(v, self.gallery_labels)?,
            "gallery.label_color" => self.gallery_label_color = Color::parse(v)?,
            "gallery.label_size" => self.gallery_label_size = len(v)?.max(1.0),
            "gallery.anim" => self.gallery_anim = len(v)?,
            _ => return Err(format!("unknown theme key '{path}'")),
        }
        Ok(())
    }

    pub fn get(&self, path: &str) -> Option<Value> {
        use Value::*;
        let n = |x: f32| Num(x as f64);
        let c = |x: Color| Str(x.to_string());
        Some(match path {
            "window.background" => c(self.window_background),
            "window.transparent" => Bool(self.window_transparent),
            "image.margin" => n(self.image_margin),
            "image.radius" => n(self.image_radius),
            "image.border.width" => n(self.image_border.width),
            "image.border.color" => c(self.image_border.color),
            "image.shadow.size" => n(self.image_shadow.size),
            "image.shadow.color" => c(self.image_shadow.color),
            "image.shadow.x" => n(self.image_shadow.x),
            "image.shadow.y" => n(self.image_shadow.y),
            "image.checker" => Bool(self.checker),
            "image.checker.a" => c(self.checker_a),
            "image.checker.b" => c(self.checker_b),
            "image.checker.size" => n(self.checker_size),
            "anim.view" => n(self.anim_view),
            "anim.wheel_zoom" => n(self.anim_wheel_zoom),
            "anim.ui" => n(self.anim_ui),
            "ui.idle" => n(self.ui_idle),
            "transition.kind" => Str(self.transition.name().into()),
            "transition.duration" => n(self.transition_duration),
            "transition.easing" => Str(self.transition_easing.name().into()),
            "font.family" => Str(self.font_family.clone()),
            "font.size" => n(self.font_size),
            "font.color" => c(self.font_color),
            "tooltip.background" => c(self.tooltip_background),
            "tooltip.color" => c(self.tooltip_color),
            "tooltip.radius" => n(self.tooltip_radius),
            "tooltip.size" => n(self.tooltip_size),
            "tooltip.delay" => n(self.tooltip_delay),
            "gallery.size" => n(self.gallery_size),
            "gallery.gap" => n(self.gallery_gap),
            "gallery.padding" => n(self.gallery_padding),
            "gallery.radius" => n(self.gallery_radius),
            "gallery.fit" => Str(if self.gallery_cover {
                "cover"
            } else {
                "contain"
            }
            .into()),
            "gallery.background" => c(self.gallery_background),
            "gallery.selected" => c(self.gallery_selected),
            "gallery.selected_width" => n(self.gallery_selected_width),
            "gallery.hover" => c(self.gallery_hover),
            "gallery.labels" => Bool(self.gallery_labels),
            "gallery.label_color" => c(self.gallery_label_color),
            "gallery.label_size" => n(self.gallery_label_size),
            "gallery.anim" => n(self.gallery_anim),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(Color::parse("#fff").unwrap().0, [1.0; 4]);
        assert_eq!(Color::parse("00000000").unwrap().0, [0.0; 4]);
        assert_eq!(Color::parse("#1e1e2e").unwrap().to_string(), "#1e1e2eff");
        assert_eq!(Color::parse("transparent").unwrap(), Color::TRANSPARENT);
        assert_eq!(Color::parse("none").unwrap(), Color::TRANSPARENT);
        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("#ééé").is_err(), "non-ascii must not panic");
        // oklch reference values: CSS red, white, black, and with alpha.
        assert_eq!(
            Color::parse("oklch(0.62796 0.25768 29.234)")
                .unwrap()
                .to_string(),
            "#ff0000ff"
        );
        assert_eq!(
            Color::parse("oklch(1 0 0)").unwrap().to_string(),
            "#ffffffff"
        );
        assert_eq!(
            Color::parse("oklch(0% 0 0)").unwrap().to_string(),
            "#000000ff"
        );
        assert_eq!(
            Color::parse("oklch(1 0 0 / 50%)").unwrap().to_string(),
            "#ffffff80"
        );
        assert_eq!(
            Color::parse("rgb(255 128 0)").unwrap().to_string(),
            "#ff8000ff"
        );
        assert_eq!(
            Color::parse("rgba(255, 0, 0, 0.5)").unwrap().to_string(),
            "#ff000080"
        );
        assert!(Color::parse("oklch(0.5 0.1)").is_err());
    }

    #[test]
    fn roundtrip_all_keys() {
        let mut t = Theme::default();
        for key in KEYS {
            let v = match t.get(key).unwrap_or_else(|| panic!("get {key}")) {
                Value::Str(s) => s,
                Value::Num(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
            };
            t.set(key, &v).unwrap_or_else(|e| panic!("{key}: {e}"));
        }
        t.set("image.checker.1", "#ff0000").unwrap();
        assert_eq!(t.checker_a, Color::parse("#f00").unwrap());
        assert!(t.set("image.nope", "1").is_err());
    }
}
