//! Key chord parsing and matching.
//!
//! Syntax (emacs/vim style): optional modifier prefixes then a key name.
//!   `n`, `N`, `+`, `C-q`, `A-Left`, `M-S-Return`, `space`, `ScrollUp`
//! Modifiers: `C-` ctrl, `A-` alt, `S-` shift, `M-`/`W-` super.
//!
//! Character keys match the produced character, so `N` already means
//! shift+n and `+` works whatever layout produces it; Shift is therefore
//! ignored for character keys. Named keys (arrows, F-keys...) honour Shift.

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl From<ModifiersState> for Mods {
    fn from(m: ModifiersState) -> Self {
        Self {
            ctrl: m.control_key(),
            alt: m.alt_key(),
            shift: m.shift_key(),
            logo: m.super_key(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyName {
    Char(String),
    /// Lowercased winit `NamedKey` debug name, e.g. `arrowleft`, or a mouse
    /// pseudo-key like `scrollup`.
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: KeyName,
}

fn alias(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "left" => "arrowleft",
        "right" => "arrowright",
        "up" => "arrowup",
        "down" => "arrowdown",
        "esc" => "escape",
        "ret" | "return" => "enter",
        "del" => "delete",
        "bs" => "backspace",
        "pgup" | "prior" => "pageup",
        "pgdn" | "next" => "pagedown",
        "ins" => "insert",
        other => other,
    }
    .to_string()
}

impl Chord {
    fn normalize(mut self) -> Self {
        if let KeyName::Char(c) = &mut self.key {
            if self.mods.shift {
                *c = c.to_uppercase();
            }
            self.mods.shift = false;
        }
        self
    }

    pub fn from_key_event(ev: &KeyEvent, mods: ModifiersState) -> Option<Self> {
        let key = match &ev.logical_key {
            Key::Character(c) if c.as_str() == " " => KeyName::Named("space".into()),
            Key::Character(c) => KeyName::Char(c.to_string()),
            Key::Named(n) => KeyName::Named(format!("{n:?}").to_ascii_lowercase()),
            _ => return None,
        };
        Some(
            Self {
                mods: mods.into(),
                key,
            }
            .normalize(),
        )
    }

    pub fn named(name: &str, mods: ModifiersState) -> Self {
        Self {
            mods: mods.into(),
            key: KeyName::Named(name.into()),
        }
    }
}

impl std::str::FromStr for Chord {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut mods = Mods::default();
        let mut rest = s;
        // A lone "-" or something like "C--" must keep its final "-".
        while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
            match rest.as_bytes()[0] {
                b'C' => mods.ctrl = true,
                b'A' => mods.alt = true,
                b'S' => mods.shift = true,
                b'M' | b'W' => mods.logo = true,
                _ => break,
            }
            rest = &rest[2..];
        }
        if rest.is_empty() {
            return Err(format!("empty key in '{s}'"));
        }
        let key = if rest.chars().count() == 1 {
            KeyName::Char(rest.into())
        } else {
            KeyName::Named(alias(rest))
        };
        Ok(Self { mods, key }.normalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn parses() {
        assert_eq!(p("n").key, KeyName::Char("n".into()));
        assert_eq!(p("S-n"), p("N"));
        let c = p("C-q");
        assert!(c.mods.ctrl && c.key == KeyName::Char("q".into()));
        assert_eq!(p("-").key, KeyName::Char("-".into()));
        assert_eq!(p("C--").key, KeyName::Char("-".into()));
        assert_eq!(p("Left").key, KeyName::Named("arrowleft".into()));
        let s = p("S-Left");
        assert!(s.mods.shift, "shift is kept for named keys");
        assert_eq!(p("ScrollUp").key, KeyName::Named("scrollup".into()));
    }
}
