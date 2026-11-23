//! Re-evaluating what the UI shows: text and value sources, input fields,
//! images to load.

use super::*;

impl Ui {
    /// Re-evaluate every text source, tooltip and `show` function.
    /// `template` expands `{key}` placeholders; `info` is passed to Lua functions.
    pub fn refresh(&mut self, template: &dyn Fn(&str) -> String, info: &LuaValue) {
        for n in &mut self.nodes {
            if let Some(src) = &n.text {
                n.text_value = match src {
                    TextSrc::Template(t) => template(t),
                    TextSrc::Func(f) => match f.call::<LuaValue>(info.clone()) {
                        Ok(LuaValue::Nil) => String::new(),
                        Ok(v) => v.to_string().unwrap_or_default(),
                        Err(e) => {
                            log::error!("ui text function: {e}");
                            String::new()
                        }
                    },
                };
            }
            if let Some(value) = n.input.as_ref().and_then(|spec| spec.value.as_ref()) {
                n.text_value = value.call::<String>(info.clone()).unwrap_or_else(|e| {
                    log::error!("ui input value: {e}");
                    String::new()
                });
            }
            if let Some(t) = &n.tooltip {
                n.tooltip_value = template(t);
            }
            if let Some(src) = &n.image {
                n.image_value = match src {
                    TextSrc::Template(t) => template(t),
                    TextSrc::Func(f) => f
                        .call::<Option<String>>(info.clone())
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                };
            }
            if let Some(rg) = &mut n.ring {
                let v = match &rg.value {
                    TextSrc::Template(t) => template(t).trim().parse::<f32>().ok(),
                    TextSrc::Func(f) => f.call::<Option<f32>>(info.clone()).ok().flatten(),
                };
                rg.now = v.unwrap_or(0.0).clamp(0.0, 1.0);
            }
            if let Some(s) = &mut n.slider {
                let v = match &s.value {
                    TextSrc::Template(t) => template(t).trim().parse::<f32>().ok(),
                    TextSrc::Func(f) => f.call::<Option<f32>>(info.clone()).ok().flatten(),
                };
                s.now = v.unwrap_or(0.0).clamp(0.0, 1.0);
            }
            if let Some(f) = &n.cursor_fn {
                n.pointer = match f.call::<LuaValue>(info.clone()) {
                    Ok(LuaValue::String(s)) => s.to_string_lossy() == "pointer",
                    Ok(LuaValue::Boolean(b)) => b,
                    _ => true,
                };
            }
            if let Some(f) = &n.show_fn {
                n.show = match f.call::<LuaValue>(info.clone()) {
                    Ok(LuaValue::Boolean(true)) => Show::Always,
                    Ok(LuaValue::Boolean(false) | LuaValue::Nil) => Show::Never,
                    Ok(LuaValue::String(s)) => {
                        parse_show(&s.to_string_lossy()).unwrap_or(Show::Always)
                    }
                    Ok(_) => Show::Always,
                    Err(e) => {
                        log::error!("ui show function: {e}");
                        Show::Always
                    }
                };
            }
        }
        for i in 0..self.nodes.len() {
            if std::mem::take(&mut self.nodes[i].fresh) {
                self.nodes[i].vis = self.target_vis(&self.nodes[i], false, false);
            }
        }
    }

    /// Show input values (or placeholders) and which one has focus.
    pub fn apply_inputs(&mut self, values: &HashMap<String, String>, focus: Option<&str>) {
        for n in &mut self.nodes {
            let (Some(spec), Some(id)) = (&n.input, &n.id) else {
                continue;
            };
            let v = values.get(id).map(String::as_str).unwrap_or("");
            n.focused = focus == Some(id.as_str());
            if spec.value.is_some() && (!n.focused || v.is_empty()) {
                // refresh() supplied the committed value. An empty edit starts
                // at the left edge so typing replaces the displayed number.
                n.input_placeholder = n.focused;
                continue;
            }
            n.input_placeholder = v.is_empty();
            n.text_value = if v.is_empty() {
                spec.placeholder.clone()
            } else {
                v.to_string()
            };
        }
    }

    /// Inputs hidden by any ancestor must release keyboard focus immediately.
    pub fn input_visible(&self, id: &str) -> bool {
        let Some(i) = self
            .nodes
            .iter()
            .position(|n| n.id.as_deref() == Some(id) && n.input.is_some())
        else {
            return false;
        };
        let mut current = Some(i);
        while let Some(i) = current {
            let n = &self.nodes[i];
            if n.squeezed_out || self.target_vis(n, true, true) <= 0.0 {
                return false;
            }
            current = n.parent;
        }
        true
    }

    /// The input element with this id: (on_submit, on_change).
    pub fn input_actions(&self, id: &str) -> Option<(Option<SliderAction>, Option<SliderAction>)> {
        let n = self.nodes.iter().find(|n| n.id.as_deref() == Some(id))?;
        let s = n.input.as_ref()?;
        Some((s.on_submit.clone(), s.on_change.clone()))
    }

    /// Image sources of visible `image` elements (for the app to load).
    pub fn wanted_images(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|n| n.image.is_some() && !n.image_value.is_empty() && n.opacity > 0.003)
            .map(|n| n.image_value.clone())
            .collect()
    }
}
