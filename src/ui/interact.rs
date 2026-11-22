//! Visibility, hover, hit-testing in paint order, overlays, scrolling and
//! sliders.

use super::*;

impl Ui {
    /// Whether an element with this id is (or is heading to be) shown.
    pub fn visible(&self, id: &str) -> bool {
        self.nodes
            .iter()
            .any(|n| n.id.as_deref() == Some(id) && self.target_vis(n, false, false) > 0.5)
    }

    /// Visibility of every element that has an id, for `wlens.visible(id)`.
    pub fn visibility(&self) -> HashMap<String, bool> {
        self.nodes
            .iter()
            .filter_map(|n| n.id.clone().map(|id| (id.clone(), self.visible(&id))))
            .collect()
    }

    /// Hidden for good (not just faded): takes no space in its parent's flow.
    pub(super) fn collapsed(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        if n.squeezed_out {
            return true;
        }
        if self.target_vis(n, true, true) <= 0.0 && n.vis < 0.01 {
            return true;
        }
        // A plain container whose children are all hidden goes too: its
        // background and padding only exist for them (e.g. a sidebar panel
        // whose content hides in image view).
        !n.children.is_empty()
            && n.text.is_none()
            && n.image.is_none()
            && n.input.is_none()
            && n.slider.is_none()
            && n.view.is_none()
            && n.children.iter().all(|&c| self.collapsed(c))
    }

    pub(super) fn target_vis(&self, n: &Node, mouse_active: bool, hovered: bool) -> f64 {
        match n.id.as_ref().and_then(|id| self.forced.get(id)) {
            Some(false) => 0.0,
            // An element with a `show` function keeps following it: `ui
            // show` only undoes an earlier `ui hide`, it can't override it.
            Some(true) if n.show != Show::Hover && n.show_fn.is_none() => 1.0,
            _ => match n.show {
                Show::Always => 1.0,
                Show::Never => 0.0,
                // Hover-revealed controls also stay while a menu is open
                // (the menu usually belongs to them).
                Show::Hover => (mouse_active || hovered || self.menu_open) as u8 as f64,
            },
        }
    }

    /// Open overlays: top-layer elements that close on a click (menus,
    /// help, dialogs), shown by `ui show`.
    pub fn open_overlays(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                n.layer_top
                    && n.on_click.is_some()
                    && n.id
                        .as_ref()
                        .is_some_and(|id| self.forced.get(id) == Some(&true))
            })
            .collect()
    }

    /// Whether `p` is on an overlay's content (its panel), not its backdrop.
    pub fn on_overlay_content(&self, i: usize, p: (f64, f64)) -> bool {
        self.nodes[i]
            .children
            .iter()
            .any(|&c| self.nodes[c].rect.contains(p))
    }

    pub fn set_forced(&mut self, id: &str, how: &str) -> Result<(), String> {
        if !self.nodes.iter().any(|n| n.id.as_deref() == Some(id)) {
            return Err(format!("no ui element with id '{id}'"));
        }
        let currently = self.visible(id);
        // With a `show` function, toggling flips the user's hide on and
        // off, whatever the function currently says.
        let follows_fn = self
            .nodes
            .iter()
            .any(|n| n.id.as_deref() == Some(id) && n.show_fn.is_some());
        let v = match how {
            "show" => true,
            "hide" => false,
            "toggle" if follows_fn => self.forced.get(id) == Some(&false),
            "toggle" => !currently,
            _ => return Err(format!("ui: expected show|hide|toggle, got '{how}'")),
        };
        self.forced.insert(id.into(), v);
        Ok(())
    }

    /// Advance fade/hover animations. Returns true while anything is moving.
    pub fn animate(
        &mut self,
        dt: f32,
        theme: &Theme,
        cursor: Option<(f64, f64)>,
        mouse_active: bool,
    ) -> bool {
        let mut moving = false;
        self.menu_open = !self.open_overlays().is_empty();
        // A visible spinner keeps frames coming (only while it's shown).
        moving |= self
            .nodes
            .iter()
            .any(|n| n.opacity > 0.003 && n.ring.as_ref().is_some_and(|r| r.spin));
        // Parents come before children, so a parent's hover is known first.
        let mut hovered_nodes = vec![false; self.nodes.len()];
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            let hovered = (cursor.is_some_and(|c| n.rect.contains(c)) && n.opacity > 0.01)
                || (n.follow_hover && n.parent.is_some_and(|p| hovered_nodes[p]));
            hovered_nodes[i] = hovered;
            let target_vis = self.target_vis(n, mouse_active, hovered);
            let target_hover = (hovered && n.hover.is_some()) as u8 as f64;
            let n = &mut self.nodes[i];
            let d = n.anim.unwrap_or(theme.anim_ui);
            if let Some(s) = &mut n.selected {
                moving |= crate::geom::anim::approach(
                    &mut s.progress,
                    f64::from(s.active == Some(true)),
                    d,
                    dt,
                    0.005,
                );
            }
            moving |= crate::geom::anim::approach(&mut n.vis, target_vis, d, dt, 0.005);
            moving |= crate::geom::anim::approach(&mut n.hover_t, target_hover, d, dt, 0.005);
        }

        // Tooltip: the topmost hovered element that has one, after a delay.
        let tipped = cursor.and_then(|c| {
            self.paint_order()
                .into_iter()
                .rev()
                .find(|&i| self.under(i, c, 0.05) && !self.nodes[i].tooltip_value.is_empty())
        });
        if tipped != self.tip.map(|t| t.0) {
            self.tip = tipped.map(|i| (i, Instant::now()));
            self.tip_vis = 0.0;
        }
        if let Some((_, since)) = self.tip {
            let due = since.elapsed().as_secs_f32() >= theme.tooltip_delay;
            moving |= !due; // keep ticking until it's time to appear
            moving |= crate::geom::anim::approach(
                &mut self.tip_vis,
                due as u8 as f64,
                theme.anim_ui,
                dt,
                0.01,
            );
        }
        moving
    }

    /// Topmost visible element under `p` (any kind), to detect hover changes.
    pub fn hover_key(&self, p: (f64, f64)) -> Option<usize> {
        self.paint_order()
            .into_iter()
            .rev()
            .find(|&i| self.under(i, p, 0.01))
    }

    /// `p` is over element `i` as drawn: visible enough, inside it, and not
    /// scrolled out of its clip.
    pub(super) fn under(&self, i: usize, p: (f64, f64), min_opacity: f32) -> bool {
        let n = &self.nodes[i];
        n.opacity > min_opacity && n.rect.contains(p) && n.clip.is_none_or(|c| c.contains(p))
    }

    /// Hover-revealed elements exist that aren't fully shown yet.
    pub fn needs_reveal(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| n.show == Show::Hover && n.vis < 0.99)
    }

    pub fn has_hover_ui(&self) -> bool {
        self.nodes
            .iter()
            .any(|n| n.show == Show::Hover && n.vis > 0.0)
    }

    /// The same base pass and deferred top pass used by draw_node.
    pub(super) fn paint_order(&self) -> Vec<usize> {
        fn visit(ui: &Ui, i: usize, top: bool, order: &mut Vec<usize>, deferred: &mut Vec<usize>) {
            let n = &ui.nodes[i];
            if n.layer_top && !top {
                deferred.push(i);
                return;
            }
            order.push(i);
            for &c in &n.children {
                visit(ui, c, top, order, deferred);
            }
        }
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut deferred = Vec::new();
        for &r in &self.roots {
            visit(self, r, false, &mut order, &mut deferred);
        }
        for i in std::mem::take(&mut deferred) {
            visit(self, i, true, &mut order, &mut deferred);
        }
        order
    }

    /// The topmost *solid* element (one with a background or a click
    /// action) under `p` takes the click; plain labels are see-through. A
    /// panel therefore swallows clicks instead of passing them to a scrim
    /// behind it.
    /// The rectangle of the `view = name` element, if it's laid out and shown.
    pub fn view_rect(&self, name: &str) -> Option<Rect> {
        let i = self
            .nodes
            .iter()
            .position(|n| n.view.as_deref() == Some(name))?;
        // Hidden somewhere up the tree -> no view region (fall back to the window).
        let mut j = Some(i);
        while let Some(k) = j {
            let n = &self.nodes[k];
            if n.show == Show::Never
                || n.id
                    .as_ref()
                    .is_some_and(|id| self.forced.get(id) == Some(&false))
            {
                return None;
            }
            j = n.parent;
        }
        let r = self.nodes[i].rect;
        (r.w >= 1.0 && r.h >= 1.0).then_some(r)
    }

    /// Scroll the scrollable element under `p` by `dy` px (positive = up /
    /// towards the start). Returns whether one took it.
    pub fn scroll_at(&mut self, p: (f64, f64), dy: f64) -> bool {
        let hit = self.paint_order().into_iter().rev().find(|&i| {
            let n = &self.nodes[i];
            n.scroll && self.under(i, p, 0.05) && n.max_scroll_y > 0.0
        });
        let Some(i) = hit else { return false };
        let n = &mut self.nodes[i];
        n.scroll_y = (n.scroll_y - dy).clamp(0.0, n.max_scroll_y);
        true
    }

    /// For a slider at node `i`: the value (0..1) at x, and its action.
    pub fn slider_at(&self, i: usize, x: f64, scale: f32) -> Option<(f32, SliderAction)> {
        let n = self.nodes.get(i)?;
        let s = n.slider.as_ref()?;
        let (pl, pr) = ((n.padding[3] * scale) as f64, (n.padding[1] * scale) as f64);
        let w = (n.rect.w - pl - pr).max(1.0);
        let v = ((x - n.rect.x - pl) / w).clamp(0.0, 1.0) as f32;
        Some((v, s.on_change.clone()?))
    }

    pub fn hit(&self, p: (f64, f64)) -> Hit {
        let top = self.paint_order().into_iter().rev().find(|&i| {
            let n = &self.nodes[i];
            self.under(i, p, 0.049)
                && !n.pass_through
                && (n.style.background.0[3] > 0.0
                    || n.on_click.is_some()
                    || n.slider.is_some()
                    || n.input.is_some())
        });
        let active = |i: usize| {
            let n = &self.nodes[i];
            n.on_click.is_some()
                || n.input.is_some()
                || n.slider.as_ref().is_some_and(|s| s.on_change.is_some())
        };
        // A click on a picture or label inside a clickable element (e.g. a
        // thumbnail in a framed button) belongs to that element, like a click inside a
        // web button. Panels (a bare background) still swallow clicks, and
        // nothing bubbles out of a panel or an overlay layer, so clicking a
        // menu's panel never reaches its backdrop.
        let clickable = top.and_then(|start| {
            if active(start) {
                return Some(start);
            }
            // Only content (a picture, a label) passes its click on; a bare
            // panel swallows it.
            let n = &self.nodes[start];
            if n.image.is_none() && n.text.is_none() {
                return None;
            }
            let mut i = start;
            while let Some(parent) = self.nodes[i].parent {
                let n = &self.nodes[parent];
                if n.layer_top {
                    return None;
                }
                if active(parent) {
                    return Some(parent);
                }
                if n.style.background.0[3] > 0.0 && !n.pass_through {
                    return None;
                }
                i = parent;
            }
            None
        });
        Hit {
            clickable,
            over_ui: top.is_some(),
            pointer: top.is_some_and(|i| active(i) && self.nodes[i].pointer),
        }
    }
}
