//! Declarative overlay UI: bars, buttons, captions.
//!
//! Lua hands us a tree of plain tables (`wlens.ui { ... }`); we parse it into
//! an arena of `Node`s, lay it out with a tiny flexbox (row/column, gap,
//! padding, `grow`), animate visibility/hover, and emit quads + text.
//!
//! All lengths in the Lua description are logical pixels.

use std::collections::HashMap;
use std::time::Instant;

use mlua::{Function, Table, Value as LuaValue};

use crate::geom::view::Rect;
use crate::lua::Action;
use crate::model::theme::{Border, Color, Shadow, Theme};
use crate::render::{Quad, TextDraw};

mod draw;
mod eval;
mod interact;
mod layout;
mod parse;
mod tree;
pub use draw::scaled_shadow;
pub use layout::shorten;
use parse::parse_show;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Default)]
struct ScrollOffset {
    shown: f64,
    target: f64,
}

impl ScrollOffset {
    fn clamp(&mut self, max: f64) -> f64 {
        self.target = self.target.clamp(0.0, max);
        self.shown = self.shown.clamp(0.0, max);
        self.shown
    }
}

pub enum ScrollInput {
    Wheel(f64),
    Pixels(f64),
}

/// Whether a top-level element keeps the image/grid out of its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserve {
    No,
    /// Always (e.g. a status bar); released while the element is hidden.
    Always,
    /// Only while the gallery grid is showing, visible or not — floating
    /// controls over an image, but no tiles hidden under them.
    Gallery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Always,
    /// Visible while the mouse is moving or hovering it (video-player style).
    Hover,
    Never,
}

/// A slider: shows a 0..1 value as a filled track; clicking or dragging
/// sets it through `on_change`.
pub struct Slider {
    /// Where the value comes from (`"{progress}"` or a function of the info).
    pub value: TextSrc,
    pub track: Color,
    pub fill: Color,
    /// A command template (`{pct}` 0–100, `{value}` 0–1) or a Lua function(value).
    pub on_change: Option<SliderAction>,
    pub(super) now: f32,
}

pub struct InputSpec {
    pub placeholder: String,
    /// Enter: a command template (`{value}`) or a Lua function(value).
    pub on_submit: Option<SliderAction>,
    /// Every edit: a Lua function(value) (or command template).
    pub on_change: Option<SliderAction>,
}

pub struct Ring {
    pub value: TextSrc,
    pub fill: Color,
    pub track: Color,
    pub thickness: f32,
    /// A spinner: a rotating arc (`value` is then the arc's length).
    pub spin: bool,
    pub(super) now: f32,
}

/// Spinner phase in turns: one turn every 0.9s.
pub fn spin_phase() -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    (t / 0.9).fract()
}

#[derive(Clone)]
pub enum SliderAction {
    Command(String),
    Lua(Function),
}

pub enum TextSrc {
    /// `{name}`-style placeholders expanded from the current image info.
    Template(String),
    /// Called with the image info table; returns the string to show.
    Func(Function),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxStyle {
    pub background: Color,
    pub radius: f32,
    pub border: Border,
    pub shadow: Shadow,
    /// Text colour; falls back to `font.color` from the theme.
    pub color: Option<Color>,
    pub opacity: f32,
}

impl Default for BoxStyle {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            radius: 0.0,
            border: Border {
                width: 0.0,
                color: Color::TRANSPARENT,
            },
            shadow: Shadow {
                size: 0.0,
                color: Color([0.0, 0.0, 0.0, 0.5]),
                x: 0.0,
                y: 2.0,
            },
            color: None,
            opacity: 1.0,
        }
    }
}

impl BoxStyle {
    fn lerp(&self, o: &Self, t: f32) -> Self {
        let l = crate::geom::anim::lerp;
        let c = |a: Option<Color>, b: Option<Color>| match (a, b) {
            (Some(a), Some(b)) => Some(a.lerp(b, t)),
            (a, b) => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        };
        Self {
            background: self.background.lerp(o.background, t),
            radius: l(self.radius, o.radius, t),
            border: Border {
                width: l(self.border.width, o.border.width, t),
                color: self.border.color.lerp(o.border.color, t),
            },
            shadow: Shadow {
                size: l(self.shadow.size, o.shadow.size, t),
                color: self.shadow.color.lerp(o.shadow.color, t),
                x: l(self.shadow.x, o.shadow.x, t),
                y: l(self.shadow.y, o.shadow.y, t),
            },
            color: c(self.color, o.color),
            opacity: l(self.opacity, o.opacity, t),
        }
    }
}

pub struct Node {
    pub id: Option<String>,
    pub anchor: (Align, Align),
    pub x: f32,
    pub y: f32,
    pub width: Size,
    pub height: Size,
    pub grow: bool,
    /// top, right, bottom, left
    pub padding: [f32; 4],
    pub gap: f32,
    pub dir: Dir,
    pub align: Align,
    /// Main-axis placement of children when none of them `grow`.
    pub justify: Align,
    pub text_align: Align,
    pub style: BoxStyle,
    pub hover: Option<BoxStyle>,
    pub text: Option<TextSrc>,
    pub font: Option<String>,
    pub font_size: Option<f32>,
    /// Font weight (400 normal, 600 semibold, 700 bold).
    pub weight: Option<u16>,
    /// Longest the text may get (logical px) before it's shortened with "…".
    pub max_width: Option<f32>,
    /// Where the "…" goes: middle keeps the start and the extension.
    pub truncate_end: bool,
    pub on_click: Option<Action>,
    pub show: Show,
    /// `show = function(img) ... end`, re-evaluated with the text.
    pub show_fn: Option<Function>,
    /// Count as hovered whenever the parent is (e.g. a chevron that lights
    /// up anywhere in its click zone).
    pub follow_hover: bool,
    /// `pass_through = true`: the mouse ignores it (a tint, a caption over
    /// the image); clicks, scrolling and drags reach whatever is below.
    /// Its children can still be clicked.
    pub pass_through: bool,
    /// Pointer cursor over this clickable element (default true).
    pub pointer: bool,
    /// `cursor = function(img) return "pointer" | "default" end`
    pub cursor_fn: Option<Function>,
    /// Per-element fade/hover duration, overriding the theme's `anim.ui`.
    pub anim: Option<f32>,
    parent: Option<usize>,
    pub tooltip: Option<String>,
    pub reserve: Reserve,
    pub slider: Option<Slider>,
    /// `ring = { value, fill, track, thickness }`: a circular progress ring
    /// drawn in the element's box (its text/children sit in the middle).
    pub ring: Option<Ring>,
    /// Where the image / thumbnail grid is drawn (`view = "main"`). It gets
    /// its rectangle from the layout like any element, so bars and sidebars
    /// are just its siblings.
    pub view: Option<String>,
    /// `image = "url or path"` (or a `{key}` template / function): drawn to
    /// fill the element, `cover` (crop, default) or `contain`.
    pub image: Option<TextSrc>,
    pub image_contain: bool,
    pub image_value: String,
    /// A text field (needs an `id`; its value lives in `wlens.input(id)`).
    pub input: Option<InputSpec>,
    pub(super) input_placeholder: bool,
    pub(super) focused: bool,
    /// Caret x offset from the text start (physical px), set by layout.
    pub(super) caret: f32,
    pub children: Vec<usize>,
    /// `children = function(img) return { … } end`: rebuilt when the current
    /// item changes (a details panel's chips, a list from metadata…).
    pub dyn_children: Option<Function>,
    /// `layer = "top"`: drawn after the rest of the UI, text included, so it
    /// covers it (menus, dialogs, toasts).
    pub layer_top: bool,
    /// A child with an `anchor`: placed over its parent (like a root over
    /// the window) instead of taking part in the parent's row/column.
    pub floating: bool,
    /// Row children flow onto new lines when they don't fit (needs a px width).
    pub wrap: bool,
    /// Content taller than the box scrolls (mouse wheel) and is clipped.
    pub scroll: bool,
    /// Cap on the height (px or %), for auto-height panels that should
    /// stop at the space available (and scroll inside).
    pub max_height: Option<Size>,
    /// Text that gives up width (shortening with "…") when its row is too
    /// full. Elements with a `max_width` shrink too.
    pub shrink: bool,
    /// Width the layout squeezed this text to (physical px), if it had to.
    pub(super) fit_width: Option<f32>,
    /// When a row is too full even after shortening text, items with a
    /// priority hide, lowest first (e.g. file size before the counter).
    pub priority: Option<i32>,
    pub(super) squeezed_out: bool,
    scroll_y: ScrollOffset,
    pub(super) content_h: f64,
    /// Where this element may draw (set by scrolling ancestors).
    pub(super) clip: Option<Rect>,
    /// `attach = "id"`: placed against that element (above it if there's
    /// room, else below), like a dropdown on its button.
    pub attach: Option<String>,
    /// Horizontal placement against the target: centred over it (default),
    /// or lined up with its start / end edge.
    pub attach_align: Align,
    dyn_key: String,
    /// Just built by a `children` function: starts shown or hidden as its
    /// `show` says, without fading.
    pub(super) fresh: bool,

    // runtime
    text_value: String,
    /// What's drawn: `text_value`, shortened with "…" if it had to fit.
    /// Kept separate so shortening never compounds and undoes itself
    /// when there's room again.
    display_text: String,
    tooltip_value: String,
    vis: f64,
    hover_t: f64,
    measured: (f32, f32),
    pub rect: Rect,
    opacity: f32,
}

/// Looks up a loaded image texture by source string.
pub type ImageLookup<'a> = dyn FnMut(&str) -> Option<(crate::render::TexId, (u32, u32))> + 'a;

#[derive(Default)]
pub struct Ui {
    /// Quads (by index in the UI quad list) that sample a texture; filled by `draw`.
    pub tex_marks: Vec<(usize, crate::render::TexId)>,
    /// Top-layer elements met during the base pass: (node, parent opacity).
    deferred: Vec<(usize, f32)>,
    drawing_top: bool,
    /// Text the arrange pass had to squeeze: (node, width) — applied by a second pass.
    squeeze: Vec<(usize, f32)>,
    /// Items an overflowing row hid this layout (by priority).
    squeeze_hide: Vec<usize>,
    /// A menu / overlay is open (see `open_overlays`), refreshed by `animate`.
    menu_open: bool,
    /// The window, from the last layout (floating elements stay inside it).
    win: Rect,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    /// `ui show|hide|toggle <id>` overrides.
    pub forced: HashMap<String, bool>,
    /// Hovered node with a tooltip, and since when.
    tip: Option<(usize, Instant)>,
    tip_vis: f64,
    tip_rect: Rect,
}

pub trait Measure {
    /// Size in physical px of `text` rendered at `size` physical px.
    fn text_size(&mut self, text: &str, family: &str, size: f32) -> (f32, f32);
}

// ---------------------------------------------------------------- parsing

/// Expand `{key}` placeholders. Unknown keys are left as-is; `{{` escapes.
pub fn expand(template: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("{{") {
            out.push('{');
            rest = r;
            continue;
        }
        match rest.find('}') {
            Some(j) => {
                let key = &rest[1..j];
                match lookup(key) {
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&rest[..=j]),
                }
                rest = &rest[j + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

impl Node {
    /// The font family to shape with, carrying the weight as "Family:600".
    pub(super) fn family(&self, theme: &Theme) -> String {
        let f = self.font.as_deref().unwrap_or(&theme.font_family);
        match self.weight {
            Some(w) => format!("{f}:{w}"),
            None => f.to_string(),
        }
    }
}

// ------------------------------------------------------------- runtime

pub struct Hit {
    /// Topmost clickable node under the cursor.
    pub clickable: Option<usize>,
    /// Cursor is over some visible UI box (so it shouldn't count as a backdrop click).
    pub over_ui: bool,
    /// Show a pointer cursor (clickable and not opted out).
    pub pointer: bool,
}
