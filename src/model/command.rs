//! The command language. Every action in wlens is a `Command`, whether it
//! comes from a keybind string, a Lua `wlens.exec(...)`, or the IPC socket.
//! Documented for users in docs/configuration.md — keep the two in sync.
//!
//! Grammar is one command per string, whitespace separated:
//!   next [n] | prev [n] | first | last | goto <n>
//!   zoom in|out|actual|<pct>%|<factor>x
//!   fit [mode] | pan <dx> <dy> | rotate <deg> | flip h|v | reset
//!   set <key> <value...> | fullscreen | reload | quit | lua <code...>
//!   ui show|hide|toggle|focus|blur <id> | copy path|name|dir|image
//!   gallery toggle|open|close|up|down|left|right|pageup|pagedown|bigger|smaller|scroll <rows>
//!   (`grid …` is an alias for `gallery …`)
//!   open | back | refresh | animation play|pause|toggle
//!   video play|pause|toggle|seek <s|+s|-s|n%>|volume <n|+n|-n>|mute

#[derive(Debug, Clone, PartialEq)]
pub enum Zoom {
    In,
    Out,
    /// Absolute zoom, 1.0 = one image pixel per physical pixel.
    To(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Next(isize),
    First,
    Last,
    /// 0-based index.
    Goto(usize),
    Zoom(Zoom),
    Fit(Option<crate::model::settings::FitMode>),
    Pan(f64, f64),
    /// Quarter turns clockwise (may be negative).
    Rotate(i32),
    FlipH,
    FlipV,
    Reset,
    Set(String, String),
    /// on / off / toggle (None).
    Fullscreen(Option<bool>),
    Reload,
    Quit,
    Lua(String),
    /// (show|hide|toggle, element id)
    Ui(String, String),
    /// Copy something about the current image to the clipboard.
    Copy(String),
    /// Thumbnail grid: (action, optional amount).
    Gallery(String, Option<f64>),
    /// Leave the current level (see `wlens.push_items`).
    Back,
    Forward,
    /// Open the current item: enter a container, or view an image.
    Open,
    /// Re-fetch the list (fires the `refresh` hook; else reloads the image).
    Refresh,
    /// Animated images: play | pause | toggle.
    Animation(String),
    /// Video: (action, argument) — play|pause|toggle|seek <s|+s|-s|n%>|volume <n|+n|-n>|mute.
    Video(String, Option<String>),
}

impl std::str::FromStr for Command {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let args: Vec<&str> = rest.split_whitespace().collect();
        // Finite numbers only (no NaN / inf sneaking into the view maths).
        let num = |s: &str| -> Result<f64, String> {
            s.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("{name}: expected a number, got '{s}'"))
        };
        // Counts stay in a sane range (and negate safely).
        let count = || -> Result<isize, String> {
            match args.first() {
                None => Ok(1),
                Some(s) => s
                    .parse::<isize>()
                    .ok()
                    .filter(|n| n.unsigned_abs() <= 1_000_000)
                    .ok_or_else(|| format!("{name}: bad count '{s}'")),
            }
        };
        let need = |n: usize| -> Result<(), String> {
            if args.len() == n {
                Ok(())
            } else {
                Err(format!(
                    "{name}: expected {n} argument(s), got {}",
                    args.len()
                ))
            }
        };

        Ok(match name {
            "next" => Command::Next(count()?),
            "prev" => Command::Next(-count()?),
            "first" => Command::First,
            "last" => Command::Last,
            "goto" => {
                need(1)?;
                // "4" or "4.0" (a Lua number that happens to be a float).
                let n = args[0]
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0)
                    .ok_or_else(|| format!("goto: bad index '{}'", args[0]))?;
                Command::Goto((n as usize).saturating_sub(1))
            }
            "zoom" => {
                need(1)?;
                let a = args[0];
                Command::Zoom(match a {
                    "in" | "+" => Zoom::In,
                    "out" | "-" => Zoom::Out,
                    "actual" | "1:1" => Zoom::To(1.0),
                    _ => {
                        let z = if let Some(p) = a.strip_suffix('%') {
                            num(p)? / 100.0
                        } else if let Some(x) = a.strip_suffix('x') {
                            num(x)?
                        } else {
                            num(a)?
                        };
                        if z <= 0.0 {
                            return Err(format!("zoom: must be above 0, got '{a}'"));
                        }
                        Zoom::To(z)
                    }
                })
            }
            "fit" => match args.first() {
                None => Command::Fit(None),
                Some(m) => Command::Fit(Some(crate::model::settings::FitMode::parse(m)?)),
            },
            "pan" => {
                need(2)?;
                Command::Pan(num(args[0])?, num(args[1])?)
            }
            "rotate" => {
                need(1)?;
                let deg: i32 = args[0]
                    .parse()
                    .map_err(|_| format!("rotate: bad angle '{}'", args[0]))?;
                if deg % 90 != 0 {
                    return Err("rotate: angle must be a multiple of 90".into());
                }
                Command::Rotate(deg / 90)
            }
            "flip" => {
                need(1)?;
                match args[0] {
                    "h" | "horizontal" => Command::FlipH,
                    "v" | "vertical" => Command::FlipV,
                    a => return Err(format!("flip: expected h or v, got '{a}'")),
                }
            }
            "reset" => Command::Reset,
            "set" => {
                let (key, value) = rest
                    .split_once(char::is_whitespace)
                    .ok_or("set: expected <key> <value>")?;
                Command::Set(key.into(), value.trim().into())
            }
            "fullscreen" => match args.first().copied() {
                None | Some("toggle") => Command::Fullscreen(None),
                Some("on") => Command::Fullscreen(Some(true)),
                Some("off") => Command::Fullscreen(Some(false)),
                Some(o) => return Err(format!("fullscreen: '{o}' (on|off|toggle)")),
            },
            "reload" => Command::Reload,
            "quit" | "q" => Command::Quit,
            "lua" => Command::Lua(rest.into()),
            "copy" => {
                need(1)?;
                if !matches!(args[0], "path" | "name" | "dir" | "image") {
                    return Err(format!(
                        "copy: expected path|name|dir|image, got '{}'",
                        args[0]
                    ));
                }
                Command::Copy(args[0].into())
            }
            "back" => Command::Back,
            "forward" => Command::Forward,
            "open" => Command::Open,
            "refresh" => Command::Refresh,
            "video" => {
                const ACTIONS: &[&str] = &["play", "pause", "toggle", "seek", "volume", "mute"];
                let a = args.first().copied().unwrap_or("toggle");
                if !ACTIONS.contains(&a) {
                    return Err(format!(
                        "video: unknown action '{a}' ({})",
                        ACTIONS.join("|")
                    ));
                }
                if matches!(a, "seek" | "volume") && args.len() != 2 {
                    return Err(format!(
                        "video {a}: needs a value (e.g. 10, +5, -5{})",
                        if a == "seek" { ", 50%" } else { "" }
                    ));
                }
                Command::Video(a.into(), args.get(1).map(|s| s.to_string()))
            }
            "animation" => {
                let a = args.first().copied().unwrap_or("toggle");
                if !matches!(a, "play" | "pause" | "toggle") {
                    return Err(format!("animation: expected play|pause|toggle, got '{a}'"));
                }
                Command::Animation(a.into())
            }
            "gallery" | "grid" => {
                const ACTIONS: &[&str] = &[
                    "toggle", "open", "close", "up", "down", "left", "right", "pageup", "pagedown",
                    "bigger", "smaller", "scroll", "flat",
                ];
                let action = args.first().copied().unwrap_or("toggle");
                if !ACTIONS.contains(&action) {
                    return Err(format!(
                        "gallery: unknown action '{action}' ({})",
                        ACTIONS.join("|")
                    ));
                }
                let amount = args.get(1).map(|a| num(a)).transpose()?;
                Command::Gallery(action.into(), amount)
            }
            "ui" => {
                need(2)?;
                if !matches!(args[0], "show" | "hide" | "toggle" | "focus" | "blur") {
                    return Err(format!(
                        "ui: expected show|hide|toggle|focus|blur, got '{}'",
                        args[0]
                    ));
                }
                Command::Ui(args[0].into(), args[1].into())
            }
            "" => return Err("empty command".into()),
            _ => return Err(format!("unknown command '{name}'")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Command {
        s.parse().unwrap()
    }

    #[test]
    fn parses() {
        assert_eq!(p("next"), Command::Next(1));
        assert_eq!(p("prev 3"), Command::Next(-3));
        assert_eq!(p("goto 1"), Command::Goto(0));
        assert_eq!(p("zoom 200%"), Command::Zoom(Zoom::To(2.0)));
        assert_eq!(p("zoom 0.5x"), Command::Zoom(Zoom::To(0.5)));
        assert_eq!(p("rotate -90"), Command::Rotate(-1));
        assert_eq!(
            p("set background #1e1e2e"),
            Command::Set("background".into(), "#1e1e2e".into())
        );
        assert_eq!(
            p("lua print('hi there')"),
            Command::Lua("print('hi there')".into())
        );
        assert!("rotate 45".parse::<Command>().is_err());
        assert!("zoom NaN".parse::<Command>().is_err());
        assert!("zoom 0".parse::<Command>().is_err());
        assert!("pan inf 0".parse::<Command>().is_err());
        assert!("prev -9223372036854775808".parse::<Command>().is_err());
        assert!("bogus".parse::<Command>().is_err());
        assert_eq!(
            p("ui toggle bar"),
            Command::Ui("toggle".into(), "bar".into())
        );
        assert_eq!(p("gallery"), Command::Gallery("toggle".into(), None));
        assert_eq!(
            p("gallery scroll -2"),
            Command::Gallery("scroll".into(), Some(-2.0))
        );
        assert!("gallery sideways".parse::<Command>().is_err());
    }
}
