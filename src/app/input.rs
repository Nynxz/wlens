//! Mouse clicks.

use super::*;

impl App {
    pub(super) fn pan_to_pointer(&mut self, at: PhysicalPosition<f64>) {
        if let Some(layer) = &mut self.current {
            self.target
                .pan_to_pointer((at.x, at.y), layer.size, self.viewport);
            layer.view = self.target;
            self.update_info();
            self.redraw();
        }
    }

    /// Run a slider/input action with a value: a command template
    /// (`{value}`, `{pct}`) or a Lua function.
    pub(super) fn run_value_action(
        &mut self,
        event_loop: &ActiveEventLoop,
        action: crate::ui::SliderAction,
        template: &[(&str, String)],
        lua_arg: mlua::Value,
    ) {
        match action {
            crate::ui::SliderAction::Command(t) => {
                let cmd = template.iter().fold(t, |acc, (k, v)| acc.replace(k, v));
                match cmd.parse() {
                    Ok(c) => self.exec(event_loop, c),
                    Err(e) => log::warn!("ui action: {e}"),
                }
            }
            crate::ui::SliderAction::Lua(f) => self.script.call_with(&f, lua_arg, "ui action"),
        }
        self.drain_queue(event_loop);
    }

    /// Keys while a text input has focus. Returns whether the key was used
    /// (all of them are, so typing never triggers bindings).
    pub(super) fn input_key(
        &mut self,
        event_loop: &ActiveEventLoop,
        ev: &winit::event::KeyEvent,
    ) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let Some(id) = self.focus.clone() else {
            return false;
        };
        if !self.ui.input_visible(&id) {
            self.focus = None;
            return false;
        }
        let ctrl = self.mods.control_key();
        let mut value = self
            .script
            .shared
            .borrow()
            .inputs
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let before = value.clone();
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.focus = None;
            }
            Key::Named(NamedKey::Enter) => {
                if let Some((Some(a), _)) = self.ui.input_actions(&id) {
                    let arg = self.script.lua_string(&value);
                    self.run_value_action(event_loop, a, &[("{value}", value.clone())], arg);
                }
            }
            Key::Named(NamedKey::Backspace) if ctrl => {
                // Delete the last word.
                let t = value.trim_end().len();
                let cut = value[..t].rfind(char::is_whitespace).map_or(0, |i| i + 1);
                value.truncate(cut);
            }
            Key::Named(NamedKey::Backspace) => {
                value.pop();
            }
            Key::Character(c) if ctrl && c.as_str() == "u" => value.clear(),
            _ if !ctrl && !self.mods.alt_key() => {
                if let Some(t) = &ev.text {
                    value.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            _ => {}
        }
        if value != before {
            self.script
                .shared
                .borrow_mut()
                .inputs
                .insert(id.clone(), value.clone());
            if let Some((_, Some(a))) = self.ui.input_actions(&id) {
                let arg = self.script.lua_string(&value);
                self.run_value_action(event_loop, a, &[("{value}", value.clone())], arg);
            }
        }
        self.text_dirty = true;
        self.redraw();
        true
    }

    /// Set a slider's value from the pointer's x (press or drag).
    pub(super) fn slide(&mut self, event_loop: &ActiveEventLoop, i: usize, x: f64) {
        let scale = self.scale;
        let Some((v, action)) = self.ui.slider_at(i, x, scale) else {
            return;
        };
        let tpl = [
            ("{pct}", format!("{:.3}", v * 100.0)),
            ("{value}", format!("{v:.5}")),
        ];
        self.run_value_action(event_loop, action, &tpl, mlua::Value::Number(v as f64));
        self.update_info();
        self.redraw();
    }

    pub(super) fn click(&mut self, event_loop: &ActiveEventLoop, at: PhysicalPosition<f64>) {
        let p = (at.x, at.y);
        // An open menu: a click anywhere but on it closes it (and does
        // nothing else), like a native menu.
        let open = self.ui.open_overlays();
        if !open.is_empty() && !open.iter().any(|&i| self.ui.on_overlay_content(i, p)) {
            for i in open {
                if let Some(id) = self.ui.nodes[i].id.clone() {
                    let _ = self.ui.set_forced(&id, "hide");
                }
            }
            self.sync_visibility();
            self.redraw();
            return;
        }
        let hit = self.ui.hit(p);
        if let Some(i) = hit.clickable
            && self.ui.nodes[i].input.is_some()
        {
            self.focus = self.ui.nodes[i].id.clone();
            self.text_dirty = true;
            self.redraw();
            return;
        }
        if self.focus.take().is_some() {
            // Clicking anywhere else leaves the text field.
            self.text_dirty = true;
            self.redraw();
        }
        if let Some(i) = hit.clickable {
            if let Some(action) = self.ui.nodes[i].on_click.clone() {
                self.run_action(event_loop, action);
            }
            return;
        }
        if hit.over_ui {
            return;
        }
        if self.gallery.active {
            // Click a thumbnail to open it.
            if let Some(i) = self
                .grid()
                .index_at(p, self.gallery.shown, self.items.len())
            {
                self.nav_dir = if i >= self.index { 1.0 } else { -1.0 };
                self.index = i;
                self.update_info();
                self.open_current(event_loop);
                self.redraw();
            }
            return;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, q)| {
            now.duration_since(t) < Duration::from_millis(350)
                && (q.x - at.x).hypot(q.y - at.y) < 8.0
        });
        self.last_click = if double { None } else { Some((now, at)) };

        let on_image = self
            .current
            .as_ref()
            .is_some_and(|l| l.view.screen_rect(l.size, self.viewport).contains(p));
        let name = if on_image {
            "clickimage"
        } else {
            "clickbackdrop"
        };
        self.trigger(event_loop, &Chord::named(name, self.mods));
        if double {
            // Zoom commands from a double-click centre on where you clicked.
            self.zoom_anchor = Some(p);
            self.trigger(event_loop, &Chord::named("doubleclick", self.mods));
            self.zoom_anchor = None;
        }
    }
}
