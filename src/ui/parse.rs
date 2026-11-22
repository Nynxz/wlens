//! Turning Lua tables into UI nodes (with friendly errors).

use super::*;
use crate::model::command::Command;

pub(super) fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(format!("ui: {}", msg.into()))
}

pub(super) fn as_f32(v: &LuaValue, key: &str) -> mlua::Result<f32> {
    match v {
        LuaValue::Integer(i) => Ok(*i as f32),
        LuaValue::Number(n) => Ok(*n as f32),
        _ => Err(err(format!(
            "'{key}' must be a number, got {}",
            v.type_name()
        ))),
    }
}

pub(super) fn as_str(v: &LuaValue, key: &str) -> mlua::Result<String> {
    match v {
        LuaValue::String(s) => Ok(s.to_str()?.to_string()),
        _ => Err(err(format!(
            "'{key}' must be a string, got {}",
            v.type_name()
        ))),
    }
}

pub(super) fn as_color(v: &LuaValue, key: &str) -> mlua::Result<Color> {
    Color::parse(&as_str(v, key)?).map_err(err)
}

pub(super) fn as_size(v: &LuaValue, key: &str) -> mlua::Result<Size> {
    match v {
        LuaValue::String(s) => {
            let s = s.to_str()?;
            if s == "auto" {
                Ok(Size::Auto)
            } else if let Some(p) = s.strip_suffix('%') {
                p.parse()
                    .map(|p: f32| Size::Percent(p / 100.0))
                    .map_err(|_| err(format!("bad {key} '{s}'")))
            } else {
                Err(err(format!(
                    "bad {key} '{s}' (number, \"50%\" or \"auto\")"
                )))
            }
        }
        _ => as_f32(v, key).map(Size::Px),
    }
}

pub(super) fn as_align(v: &LuaValue, key: &str) -> mlua::Result<Align> {
    Ok(match as_str(v, key)?.as_str() {
        "start" | "left" | "top" => Align::Start,
        "center" | "middle" => Align::Center,
        "end" | "right" | "bottom" => Align::End,
        s => return Err(err(format!("bad {key} '{s}' (start|center|end)"))),
    })
}

pub(super) fn parse_show(s: &str) -> mlua::Result<Show> {
    Ok(match s {
        "always" => Show::Always,
        "hover" => Show::Hover,
        "never" | "hidden" => Show::Never,
        s => {
            return Err(err(format!(
                "bad show '{s}' (always|hover|never, a bool, or a function)"
            )));
        }
    })
}

pub(super) fn as_action(v: &LuaValue, key: &str) -> mlua::Result<Action> {
    match v {
        LuaValue::String(s) => Ok(Action::Command(
            s.to_str()?.parse::<Command>().map_err(err)?,
        )),
        LuaValue::Function(f) => Ok(Action::Lua(f.clone())),
        _ => Err(err(format!("'{key}' must be a command string or function"))),
    }
}

/// Apply style keys from `t` onto `s`. Returns false if `key` isn't a style key.
pub(super) fn style_key(s: &mut BoxStyle, key: &str, v: &LuaValue) -> mlua::Result<bool> {
    match key {
        "background" | "bg" => s.background = as_color(v, key)?,
        "radius" => s.radius = as_f32(v, key)?,
        "color" => s.color = Some(as_color(v, key)?),
        "opacity" => s.opacity = as_f32(v, key)?.clamp(0.0, 1.0),
        "border_color" => s.border.color = as_color(v, key)?,
        "border" => match v {
            LuaValue::Table(t) => {
                for pair in t.pairs::<String, LuaValue>() {
                    let (k, v) = pair?;
                    match k.as_str() {
                        "width" => s.border.width = as_f32(&v, "border.width")?,
                        "color" => s.border.color = as_color(&v, "border.color")?,
                        _ => return Err(err(format!("unknown border key '{k}'"))),
                    }
                }
            }
            _ => s.border.width = as_f32(v, key)?,
        },
        "shadow" => match v {
            LuaValue::Table(t) => {
                for pair in t.pairs::<String, LuaValue>() {
                    let (k, v) = pair?;
                    match k.as_str() {
                        "size" => s.shadow.size = as_f32(&v, "shadow.size")?,
                        "color" => s.shadow.color = as_color(&v, "shadow.color")?,
                        "x" => s.shadow.x = as_f32(&v, "shadow.x")?,
                        "y" => s.shadow.y = as_f32(&v, "shadow.y")?,
                        _ => return Err(err(format!("unknown shadow key '{k}'"))),
                    }
                }
            }
            _ => s.shadow.size = as_f32(v, key)?,
        },
        _ => return Ok(false),
    }
    Ok(true)
}

impl Ui {
    pub fn parse(roots: Table) -> mlua::Result<Self> {
        let mut ui = Ui::default();
        for v in roots.sequence_values::<LuaValue>() {
            let i = ui.parse_node(v?, true)?;
            ui.roots.push(i);
        }
        Ok(ui)
    }

    pub(super) fn parse_node(&mut self, v: LuaValue, root: bool) -> mlua::Result<usize> {
        let t = match v {
            LuaValue::Table(t) => t,
            // Bare strings are text labels: { "hello", "{name}" }
            LuaValue::String(s) => {
                let lua_text = s.to_str()?.to_string();
                return Ok(self.push(Node::new(Some(TextSrc::Template(lua_text)))));
            }
            LuaValue::Function(f) => return Ok(self.push(Node::new(Some(TextSrc::Func(f))))),
            other => {
                return Err(err(format!(
                    "expected a table, string or function, got {}",
                    other.type_name()
                )));
            }
        };

        let mut n = Node::new(None);
        let mut hover: Option<Table> = None;
        let mut selected: Option<Table> = None;
        let mut children = Vec::new();
        for pair in t.pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair?;
            let key = match k {
                LuaValue::Integer(i) => {
                    children.push((i, v));
                    continue;
                }
                LuaValue::String(s) => s.to_str()?.to_string(),
                _ => return Err(err("table keys must be strings")),
            };
            if style_key(&mut n.style, &key, &v)? {
                continue;
            }
            match key.as_str() {
                "id" => n.id = Some(as_str(&v, &key)?),
                "anchor" => {
                    let a = as_str(&v, &key)?;
                    let (vert, horiz) = match a.split_once(['-', '_']) {
                        Some((a, b)) => (a, b),
                        None if matches!(a.as_str(), "left" | "right") => ("center", a.as_str()),
                        None => (a.as_str(), "center"),
                    };
                    // Lua spells it `top-right`; we store (horizontal, vertical).
                    let side = |s: &str| -> mlua::Result<Align> {
                        Ok(match s {
                            "top" | "left" => Align::Start,
                            "center" | "middle" => Align::Center,
                            "bottom" | "right" => Align::End,
                            _ => {
                                return Err(err(format!(
                                    "bad anchor '{a}' (e.g. top-left, bottom, center, right)"
                                )));
                            }
                        })
                    };
                    n.anchor = (side(horiz)?, side(vert)?);
                }
                "x" => n.x = as_f32(&v, &key)?,
                "y" => n.y = as_f32(&v, &key)?,
                "width" => n.width = as_size(&v, &key)?,
                "height" => n.height = as_size(&v, &key)?,
                "grow" => n.grow = matches!(v, LuaValue::Boolean(true)),
                "gap" => n.gap = as_f32(&v, &key)?,
                "padding" => {
                    n.padding = match &v {
                        LuaValue::Table(p) => {
                            let p: Vec<f32> =
                                p.sequence_values::<f32>().collect::<mlua::Result<_>>()?;
                            match p[..] {
                                [a] => [a; 4],
                                [v, h] => [v, h, v, h],
                                [t, r, b, l] => [t, r, b, l],
                                _ => return Err(err("padding takes 1, 2 or 4 numbers")),
                            }
                        }
                        _ => [as_f32(&v, &key)?; 4],
                    }
                }
                "direction" | "dir" => {
                    n.dir = match as_str(&v, &key)?.as_str() {
                        "row" | "horizontal" => Dir::Row,
                        "column" | "vertical" => Dir::Column,
                        s => return Err(err(format!("bad direction '{s}' (row|column)"))),
                    }
                }
                "align" => n.align = as_align(&v, &key)?,
                "justify" => n.justify = as_align(&v, &key)?,
                "tooltip" => n.tooltip = Some(as_str(&v, &key)?),
                "follow_hover" => n.follow_hover = matches!(v, LuaValue::Boolean(true)),
                "pass_through" => n.pass_through = matches!(v, LuaValue::Boolean(true)),
                "anim" => n.anim = Some(as_f32(&v, &key)?.max(0.0)),
                "cursor" if matches!(v, LuaValue::Function(_)) => {
                    if let LuaValue::Function(f) = v {
                        n.cursor_fn = Some(f);
                    }
                }
                "cursor" => {
                    n.pointer = match as_str(&v, &key)?.as_str() {
                        "pointer" => true,
                        "default" => false,
                        s => return Err(err(format!("bad cursor '{s}' (pointer|default)"))),
                    }
                }
                "text_align" => n.text_align = as_align(&v, &key)?,
                "hover" => match v {
                    LuaValue::Table(h) => hover = Some(h),
                    _ => return Err(err("'hover' must be a table of style keys")),
                },
                "selected" => match v {
                    LuaValue::Table(t) => selected = Some(t),
                    _ => return Err(err("'selected' must be a table with a 'when' function")),
                },
                "stack" => n.stack = matches!(v, LuaValue::Boolean(true)),
                "text" => {
                    n.text = Some(match v {
                        LuaValue::Function(f) => TextSrc::Func(f),
                        _ => TextSrc::Template(as_str(&v, &key)?),
                    })
                }
                "font" => n.font = Some(as_str(&v, &key)?),
                "weight" => {
                    n.weight = Some(match &v {
                        LuaValue::Integer(i) => (*i).clamp(100, 900) as u16,
                        LuaValue::Number(f) => (*f as i64).clamp(100, 900) as u16,
                        _ => match as_str(&v, &key)?.as_str() {
                            "light" => 300,
                            "normal" | "regular" => 400,
                            "medium" => 500,
                            "semibold" => 600,
                            "bold" => 700,
                            s => {
                                return Err(err(format!(
                                    "bad weight '{s}' (light|normal|medium|semibold|bold or 100-900)"
                                )));
                            }
                        },
                    })
                }
                "max_width" => n.max_width = Some(as_f32(&v, &key)?),
                "truncate" => {
                    n.truncate_end = match as_str(&v, &key)?.as_str() {
                        "end" => true,
                        "middle" => false,
                        s => return Err(err(format!("bad truncate '{s}' (middle|end)"))),
                    }
                }
                "font_size" | "size" => n.font_size = Some(as_f32(&v, &key)?),
                "on_click" => n.on_click = Some(as_action(&v, &key)?),
                "attach" => n.attach = Some(as_str(&v, &key)?),
                "wrap" => n.wrap = matches!(v, LuaValue::Boolean(true)),
                "scroll" => n.scroll = matches!(v, LuaValue::Boolean(true)),
                "max_height" => n.max_height = Some(as_size(&v, &key)?),
                "shrink" => n.shrink = matches!(v, LuaValue::Boolean(true)),
                "priority" => n.priority = Some(as_f32(&v, &key)? as i32),
                "attach_align" => n.attach_align = as_align(&v, &key)?,
                "layer" => {
                    n.layer_top = match as_str(&v, &key)?.as_str() {
                        "top" => true,
                        "base" => false,
                        s => return Err(err(format!("bad layer '{s}' (base|top)"))),
                    }
                }
                "children" => match v {
                    LuaValue::Function(f) => n.dyn_children = Some(f),
                    _ => {
                        return Err(err(
                            "'children' must be a function(img) returning a list of elements",
                        ));
                    }
                },
                "input" => {
                    let t = match v {
                        LuaValue::Table(t) => t,
                        _ => {
                            return Err(err(
                                "'input' is a table: { placeholder, on_submit, on_change }",
                            ));
                        }
                    };
                    let act = |k: &str| -> mlua::Result<Option<SliderAction>> {
                        Ok(match t.get::<LuaValue>(k)? {
                            LuaValue::Nil => None,
                            LuaValue::Function(f) => Some(SliderAction::Lua(f)),
                            v => Some(SliderAction::Command(as_str(&v, k)?)),
                        })
                    };
                    n.input = Some(InputSpec {
                        placeholder: t.get::<Option<String>>("placeholder")?.unwrap_or_default(),
                        on_submit: act("on_submit")?,
                        on_change: act("on_change")?,
                    });
                }
                "image" => {
                    n.image = Some(match v {
                        LuaValue::Function(f) => TextSrc::Func(f),
                        _ => TextSrc::Template(as_str(&v, &key)?),
                    })
                }
                "fit" => {
                    n.image_contain = match as_str(&v, &key)?.as_str() {
                        "contain" => true,
                        "cover" => false,
                        s => return Err(err(format!("bad fit '{s}' (cover|contain)"))),
                    }
                }
                "view" => {
                    let v = as_str(&v, &key)?;
                    if v != "main" {
                        return Err(err(format!("view: '{v}' (only \"main\" for now)")));
                    }
                    n.view = Some(v);
                }
                "ring" => {
                    let t = match v {
                        LuaValue::Table(t) => t,
                        _ => {
                            return Err(err(
                                "'ring' is a table: { value = fn | \"{key}\", fill, track, thickness }",
                            ));
                        }
                    };
                    let value = match t.get::<LuaValue>("value")? {
                        LuaValue::Function(f) => TextSrc::Func(f),
                        v @ LuaValue::String(_) => TextSrc::Template(as_str(&v, "ring.value")?),
                        LuaValue::Nil if t.get::<Option<bool>>("spin")?.unwrap_or(false) => {
                            TextSrc::Template("0.3".into()) // a spinner's arc length
                        }
                        _ => return Err(err("ring.value: a \"{key}\" template or a function")),
                    };
                    let col = |k: &str, d: &str| -> mlua::Result<Color> {
                        match t.get::<LuaValue>(k)? {
                            LuaValue::Nil => Ok(Color::parse(d).unwrap()),
                            v => as_color(&v, k),
                        }
                    };
                    n.ring = Some(Ring {
                        value,
                        fill: col("fill", "#ffffff")?,
                        track: col("track", "#ffffff33")?,
                        thickness: t.get::<Option<f32>>("thickness")?.unwrap_or(3.0),
                        spin: t.get::<Option<bool>>("spin")?.unwrap_or(false),
                        now: 0.0,
                    });
                }
                "slider" => {
                    let t = match v {
                        LuaValue::Table(t) => t,
                        _ => {
                            return Err(err(
                                "'slider' is a table: { value = \"{progress}\", on_change = \"video seek {pct}%\", track, fill }",
                            ));
                        }
                    };
                    let value = match t.get::<LuaValue>("value")? {
                        LuaValue::Function(f) => TextSrc::Func(f),
                        v @ LuaValue::String(_) => TextSrc::Template(as_str(&v, "slider.value")?),
                        _ => return Err(err("slider.value: a \"{key}\" template or a function")),
                    };
                    let on_change = match t.get::<LuaValue>("on_change")? {
                        LuaValue::Nil => None,
                        LuaValue::Function(f) => Some(SliderAction::Lua(f)),
                        v => Some(SliderAction::Command(as_str(&v, "slider.on_change")?)),
                    };
                    let col = |k: &str, d: &str| -> mlua::Result<Color> {
                        match t.get::<LuaValue>(k)? {
                            LuaValue::Nil => Ok(Color::parse(d).unwrap()),
                            v => as_color(&v, k),
                        }
                    };
                    n.slider = Some(Slider {
                        value,
                        track: col("track", "#ffffff33")?,
                        fill: col("fill", "#ffffff")?,
                        on_change,
                        now: 0.0,
                    });
                }
                "show" => match v {
                    LuaValue::Function(f) => n.show_fn = Some(f),
                    LuaValue::Boolean(b) => n.show = if b { Show::Always } else { Show::Never },
                    _ => n.show = parse_show(&as_str(&v, &key)?)?,
                },
                "reserve" => {
                    n.reserve = match &v {
                        LuaValue::Boolean(true) => Reserve::Always,
                        LuaValue::Boolean(false) | LuaValue::Nil => Reserve::No,
                        LuaValue::String(s) if s.to_str()?.as_ref() as &str == "gallery" => {
                            Reserve::Gallery
                        }
                        _ => return Err(err("'reserve' is true, false or \"gallery\"")),
                    };
                }
                _ => return Err(err(format!("unknown key '{key}'"))),
            }
        }
        if let Some(t) = selected {
            let when = t.get::<Function>("when")?;
            let mut style = n.style.clone();
            let mut justify = n.justify;
            for pair in t.pairs::<String, LuaValue>() {
                let (k, v) = pair?;
                if k == "justify" {
                    justify = as_align(&v, &k)?;
                } else if k != "when" && !style_key(&mut style, &k, &v)? {
                    return Err(err(format!("unknown selected style key '{k}'")));
                }
            }
            n.selected = Some(Box::new(SelectedStyle {
                when,
                style,
                justify,
                active: None,
                progress: 0.0,
            }));
        }
        if let Some(h) = hover {
            let mut hs = n.style.clone();
            for pair in h.pairs::<String, LuaValue>() {
                let (k, v) = pair?;
                if !style_key(&mut hs, &k, &v)? {
                    return Err(err(format!(
                        "'{k}' is not a style key (allowed in hover: background, radius, color, opacity, border, shadow)"
                    )));
                }
            }
            n.hover = Some(hs);
        }
        if n.on_click.is_some() && n.hover.is_none() {
            // Clickable things should react a little even if unstyled.
            let mut hs = n.style.clone();
            hs.opacity = 1.0;
            n.hover = Some(hs);
        }
        if !root && t.contains_key("anchor")? {
            n.floating = true;
        }
        if n.reserve != Reserve::No && !root && !n.floating {
            return Err(err("'reserve' applies to top-level or anchored elements"));
        }
        if n.input.is_some() && n.id.is_none() {
            return Err(err(
                "an 'input' element needs an 'id' (its value is wlens.input(id))",
            ));
        }
        n.vis = if n.show == Show::Never { 0.0 } else { 1.0 };
        let idx = self.push(n);
        // pairs() doesn't promise index order (tables built with t[#t+1] can
        // keep their items in the hash part), so sort.
        children.sort_by_key(|(i, _)| *i);
        for (_, c) in children {
            let ci = self.parse_node(c, false)?;
            self.nodes[ci].parent = Some(idx);
            self.nodes[idx].children.push(ci);
        }
        Ok(idx)
    }

    pub(super) fn push(&mut self, n: Node) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }
}

impl Node {
    pub(super) fn new(text: Option<TextSrc>) -> Self {
        Self {
            id: None,
            anchor: (Align::Start, Align::Start),
            x: 0.0,
            y: 0.0,
            width: Size::Auto,
            height: Size::Auto,
            grow: false,
            padding: [0.0; 4],
            gap: 0.0,
            dir: Dir::Row,
            align: Align::Center,
            justify: Align::Start,
            text_align: Align::Center,
            style: BoxStyle::default(),
            hover: None,
            selected: None,
            text,
            font: None,
            font_size: None,
            weight: None,
            max_width: None,
            truncate_end: false,
            on_click: None,
            show: Show::Always,
            show_fn: None,
            follow_hover: false,
            pass_through: false,
            pointer: true,
            cursor_fn: None,
            anim: None,
            parent: None,
            tooltip: None,
            reserve: Reserve::No,
            slider: None,
            ring: None,
            view: None,
            image: None,
            image_contain: false,
            image_value: String::new(),
            input: None,
            input_placeholder: false,
            focused: false,
            caret: 0.0,
            dyn_children: None,
            dyn_key: String::new(),
            fresh: false,
            layer_top: false,
            floating: false,
            wrap: false,
            stack: false,
            scroll: false,
            max_height: None,
            shrink: false,
            priority: None,
            squeezed_out: false,
            fit_width: None,
            scroll_y: 0.0,
            max_scroll_y: 0.0,
            clip: None,
            attach: None,
            attach_align: Align::Center,
            children: Vec::new(),
            text_value: String::new(),
            display_text: String::new(),
            tooltip_value: String::new(),
            vis: 1.0,
            hover_t: 0.0,
            measured: (0.0, 0.0),
            rect: Rect::default(),
            opacity: 0.0,
        }
    }
}
