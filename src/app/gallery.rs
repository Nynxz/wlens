//! The thumbnail grid: geometry, commands, thumbnail loading, drawing.

use super::*;

impl App {
    pub(super) fn grid(&self) -> Grid {
        let th = &self.script.shared.borrow().settings.theme;
        let s = self.scale as f64;
        let label = if th.gallery_labels {
            th.gallery_label_size as f64 * 1.9 * s
        } else {
            0.0
        };
        Grid::new(
            self.viewport,
            th.gallery_size as f64 * s,
            th.gallery_gap as f64 * s,
            th.gallery_padding as f64 * s,
            label,
        )
        .with_insets(self.grid_insets.0, self.grid_insets.1)
    }

    pub(super) fn more(&self) -> bool {
        self.script.shared.borrow().more
    }

    /// Item count for scroll limits: one extra row while more may load,
    /// so the "Loading more…" row can be scrolled into view.
    pub(super) fn grid_len(&self, g: &Grid) -> usize {
        let n = self.items.len();
        if self.more() && n > 0 {
            n.div_ceil(g.cols) * g.cols + g.cols
        } else {
            n
        }
    }

    pub(super) fn ensure_selection_visible(&mut self) {
        let g = self.grid();
        let len = self.grid_len(&g);
        self.gallery.scroll = g.scroll_to_show(self.index, self.gallery.scroll, len);
    }

    pub(super) fn set_gallery(&mut self, on: bool) {
        if on == self.gallery.active {
            return;
        }
        if !on {
            self.forward_image = false;
            self.forward_levels.clear();
        }
        self.gallery.active = on;
        if on {
            self.update_browser_view();
            self.ensure_selection_visible();
            if self.gallery.t < 0.01 {
                // Opening fresh: jump straight to the selection.
                self.gallery.shown = self.gallery.scroll;
            }
            self.gallery.requested = None;
        }
        self.sync();
        self.script.emit("mode");
    }

    pub(super) fn gallery_command(&mut self, action: &str, amount: Option<f64>) {
        let n = self.items.len();
        let g = self.grid();
        let i = self.index as isize;
        let cols = g.cols as isize;
        match action {
            // Image view -> grid; grid -> the selected image. A selected
            // folder / gallery has no image view, so the grid stays (opening
            // it is `open` / Return / a click).
            "toggle" if self.gallery.active => {
                if self.items.get(self.index).is_some_and(|i| !i.container) {
                    self.set_gallery(false);
                }
            }
            "toggle" => self.set_gallery(true),
            "open" => self.set_gallery(true),
            "close" if self.items.get(self.index).is_some_and(|i| i.container) => {
                self.pending_open = true;
            }
            "close" => self.set_gallery(false),
            "bigger" | "smaller" => {
                let f = amount.unwrap_or(1.15);
                let mut sh = self.script.shared.borrow_mut();
                let t = &mut sh.settings.theme;
                let size = if action == "bigger" {
                    t.gallery_size * f as f32
                } else {
                    t.gallery_size / f as f32
                };
                t.gallery_size = size.clamp(48.0, 1024.0);
                drop(sh);
                self.gallery.requested = None;
                self.ensure_selection_visible();
            }
            "scroll" => {
                let rows = amount.unwrap_or(1.0);
                let row_h = g.cell + g.label + g.gap;
                self.gallery.scroll = (self.gallery.scroll + rows * row_h)
                    .clamp(0.0, g.max_scroll(self.grid_len(&g)));
            }
            _ if !self.gallery.active => {}
            "left" => self.navigate(i - 1),
            "right" => self.navigate(i + 1),
            "up" if i >= cols => self.navigate(i - cols),
            "down" if i + cols < n as isize => self.navigate(i + cols),
            // last row is shorter: jump to the final item rather than nowhere
            "down" if i / cols < (n as isize - 1) / cols => self.navigate(n as isize - 1),
            "pageup" => self.navigate((i - cols * g.page_rows() as isize).max(i % cols)),
            "pagedown" => self.navigate((i + cols * g.page_rows() as isize).min(n as isize - 1)),
            _ => {}
        }
    }

    /// Ask the loader for thumbnails around what's visible (only when that changes).
    pub(super) fn request_thumbs(&mut self) {
        if !self.gallery.active || self.items.is_empty() {
            return;
        }
        let g = self.grid();
        let n = self.items.len();
        let level = crate::io::thumbs::level_for(g.cell.ceil() as u32);
        let range = g.visible(n, self.gallery.scroll, 2);
        // Within two rows of the end: let scripts load more (infinite scroll).
        self.maybe_end_reached(range.end >= n);
        let n = self.items.len();
        let range = g.visible(n, self.gallery.scroll, 2);
        let key = (range.clone(), level, n);
        if self.gallery.requested.as_ref() == Some(&key) {
            return;
        }
        self.gallery.requested = Some(key);
        let Some(loader) = &self.loader else { return };
        for job in loader.take_queued(|j| matches!(j, Job::Thumb(..))) {
            if let Job::Thumb(s, _, l) = job {
                self.gallery.pending.remove(&(s, l));
            }
        }
        // Visible cells first, nearest the selection first.
        let mut order: Vec<usize> = range.collect();
        order.sort_by_key(|&i| i.abs_diff(self.index));
        let mut jobs = Vec::new();
        for i in order {
            if self.items[i].container && self.items[i].thumb.is_none() {
                continue;
            }
            if self.items[i].is_video()
                && self.items[i].src.is_remote()
                && self.items[i].thumb.is_none()
            {
                // A grid cell must not download an entire remote video.
                continue;
            }
            let src = &self.items[i].src;
            let have = self
                .gallery
                .thumbs
                .get(src)
                .is_some_and(|t| t.level >= level);
            let key = (src.clone(), level);
            if have || self.gallery.failed.contains(src) || self.gallery.pending.contains(&key) {
                continue;
            }
            self.gallery.pending.insert(key);
            jobs.push(Job::Thumb(src.clone(), self.items[i].thumb.clone(), level));
        }
        loader.request(jobs);
    }

    /// Load what `image` elements in the UI show (through the thumbnail cache).
    pub(super) fn request_ui_images(&mut self) {
        let wanted = self.ui.wanted_images();
        if wanted.is_empty() {
            return;
        }
        let level = crate::io::thumbs::level_for(512);
        let mut jobs = Vec::new();
        for s in wanted {
            let src = Source::parse(&s);
            let key = (src.clone(), level);
            if self.gallery.thumbs.contains_key(&src)
                || self.gallery.failed.contains(&src)
                || self.gallery.pending.contains(&key)
            {
                continue;
            }
            self.gallery.pending.insert(key);
            jobs.push(Job::Thumb(src, None, level));
        }
        if let Some(loader) = &self.loader
            && !jobs.is_empty()
        {
            loader.request(jobs);
        }
    }

    pub(super) fn on_thumb(&mut self, src: Source, result: ThumbResult) {
        let level =
            |t: &crate::io::thumbs::Thumb| crate::io::thumbs::level_for(t.size.0.max(t.size.1));
        match result {
            Ok(t) => {
                let lvl = level(&t);
                self.gallery.pending.retain(|(s, _)| *s != src);
                let Some(r) = &mut self.renderer else { return };
                let tex = r.upload(&t.rgba, t.size);
                let frame = self.gallery.frame;
                if let Some(old) = self.gallery.thumbs.insert(
                    src.clone(),
                    ThumbTex {
                        tex,
                        size: t.size,
                        orig: t.orig_size,
                        level: lvl,
                        used: frame,
                    },
                ) {
                    r.release(old.tex);
                }
                // Evict the least recently drawn thumbnails beyond the budget.
                if self.gallery.thumbs.len() > MAX_GPU_THUMBS {
                    let mut by_age: Vec<(u64, Source)> = self
                        .gallery
                        .thumbs
                        .iter()
                        .map(|(s, t)| (t.used, s.clone()))
                        .collect();
                    by_age.sort_by_key(|(u, _)| *u);
                    for (_, s) in by_age
                        .into_iter()
                        .take(self.gallery.thumbs.len() - MAX_GPU_THUMBS)
                    {
                        if let Some(t) = self.gallery.thumbs.remove(&s) {
                            r.release(t.tex);
                        }
                    }
                }
                if self.current_src() == Some(&src) {
                    self.update_info();
                }
                self.redraw();
            }
            Err(e) => {
                log::debug!("thumbnail {}: {e}", src.as_string());
                self.gallery.pending.retain(|(s, _)| *s != src);
                self.gallery.failed.insert(src);
                self.redraw();
            }
        }
    }

    pub(super) fn draw_gallery(
        &mut self,
        op: f32,
        quads: &mut Vec<(Option<TexId>, Quad)>,
        texts: &mut Vec<TextDraw>,
    ) {
        let th = self.script.shared.borrow().settings.theme.clone();
        let g = self.grid();
        let scale = self.scale;
        let n = self.items.len();
        let scroll = self.gallery.shown;
        self.gallery.frame += 1;
        let frame = self.gallery.frame;
        let radius = th.gallery_radius * scale;
        let none = Border {
            width: 0.0,
            color: Color::TRANSPARENT,
        };
        let hovered = self.cursor.and_then(|c| g.index_at((c.x, c.y), scroll, n));
        // Grow in slightly as it fades in.
        let zoom = lerp(0.97, 1.0, op) as f64;
        let c = g.vp.center();
        let z = |r: Rect| {
            Rect::new(
                c.0 + (r.x - c.0) * zoom,
                c.1 + (r.y - c.1) * zoom,
                r.w * zoom,
                r.h * zoom,
            )
        };

        if self.more() && n == 0 {
            // A list that hasn't arrived yet: a spinner in the middle.
            let (cx, cy) = g.vp.center();
            let d = 28.0 * scale as f64;
            let c = th.gallery_label_color;
            quads.push((
                None,
                Quad::ring(
                    Rect::new(cx - d / 2.0, cy - d, d, d),
                    3.0 * scale,
                    0.3,
                    crate::ui::spin_phase(),
                    c.with_alpha(c.0[3] * 0.9),
                    c.with_alpha(c.0[3] * 0.15),
                    op,
                ),
            ));
            texts.push(TextDraw {
                text: "Loading…".into(),
                family: th.font_family.clone(),
                size: th.font_size * scale,
                rect: Rect::new(g.vp.x, cy + 6.0 * scale as f64, g.vp.w, 24.0 * scale as f64),
                align: Align::Center,
                color: c.with_alpha(c.0[3] * op * 0.8),
                clip: None,
            });
        }
        if self.more() && n > 0 {
            // The row after the last tile: there's more on the way.
            let row_start = n.div_ceil(g.cols) * g.cols;
            let r = g.rect(row_start, scroll);
            if r.y < g.vp.y + g.vp.h && r.y + r.h > g.vp.y {
                let c = th.gallery_label_color;
                texts.push(TextDraw {
                    text: "Loading more…".into(),
                    family: th.font_family.clone(),
                    size: th.font_size * scale,
                    rect: Rect::new(g.vp.x, r.y, g.vp.w, r.h.min(80.0 * scale as f64)),
                    align: Align::Center,
                    color: c.with_alpha(c.0[3] * op * 0.8),
                    clip: None,
                });
            }
        }
        for i in g.visible(n, scroll, 0) {
            let r = z(g.rect(i, scroll));
            let container = self.items[i].container;
            if container {
                // Stacked cards peeking out above: "this holds more".
                let s = scale as f64;
                for (k, inset) in [(2.0, 10.0), (1.0, 5.0)] {
                    // Only a strip peeks out, so translucent tiles don't stack up into grey.
                    let peek = k * 3.5 * s;
                    let card = Rect::new(
                        r.x + inset * s,
                        r.y - peek,
                        r.w - 2.0 * inset * s,
                        peek + 2.0 * s,
                    );
                    let shade = th
                        .gallery_background
                        .with_alpha(th.gallery_background.0[3] * (0.5 + 0.25 * (2.0 - k as f32)));
                    quads.push((None, Quad::solid(card, shade, radius, none, op)));
                }
            }
            quads.push((
                None,
                Quad::solid(r, th.gallery_background, radius, none, op),
            ));
            let src = &self.items[i].src;
            let has_thumb = self.gallery.thumbs.contains_key(src);
            if container
                && !has_thumb
                && let Some(icon) = &self.items[i].icon
            {
                let c = th.gallery_label_color;
                texts.push(TextDraw {
                    text: icon.clone(),
                    family: th.font_family.clone(),
                    size: (r.h * 0.32) as f32,
                    rect: r,
                    align: Align::Center,
                    color: c.with_alpha(c.0[3] * op * 0.8),
                    clip: None,
                });
            }
            if let Some(t) = self.gallery.thumbs.get_mut(src) {
                t.used = frame;
                let fit = |a: f64, b: f64| if th.gallery_cover { a.max(b) } else { a.min(b) };
                let v = View {
                    zoom: fit(r.w / t.size.0 as f64, r.h / t.size.1 as f64),
                    ..View::default()
                };
                let rect = if th.gallery_cover {
                    r
                } else {
                    v.screen_rect(t.size, r)
                };
                let q = Quad::image(
                    rect,
                    v.screen_to_uv(t.size, r),
                    radius,
                    none,
                    th.checker,
                    op,
                );
                quads.push((Some(t.tex), q));
            }
            if container
                && has_thumb
                && let Some(icon) = &self.items[i].icon
            {
                let side = (24.0 * scale as f64).min(r.w).min(r.h);
                let badge = Rect::new(r.x + 3.0, r.y + 3.0, side, side);
                quads.push((
                    None,
                    Quad::solid(
                        badge,
                        crate::model::theme::Color([0.0, 0.0, 0.0, 0.72]),
                        radius,
                        none,
                        op,
                    ),
                ));
                texts.push(TextDraw {
                    text: icon.clone(),
                    family: th.font_family.clone(),
                    size: (side * 0.68) as f32,
                    rect: badge,
                    align: Align::Center,
                    color: crate::model::theme::Color([1.0, 1.0, 1.0, op]),
                    clip: None,
                });
            }
            let ring = if i == self.index {
                Some((th.gallery_selected, th.gallery_selected_width))
            } else if hovered == Some(i) {
                Some((th.gallery_hover, th.gallery_selected_width.min(1.5)))
            } else {
                None
            };
            if let Some((color, width)) = ring {
                let w = width * scale;
                let pad = w as f64 + 1.5 * scale as f64;
                let outer = Rect::new(r.x - pad, r.y - pad, r.w + 2.0 * pad, r.h + 2.0 * pad);
                let border = Border { width: w, color };
                quads.push((
                    None,
                    Quad::solid(outer, Color::TRANSPARENT, radius + pad as f32, border, op),
                ));
            }
            if let Some(count) = &self.items[i].count {
                let s = scale as f64;
                let text = count.clone();
                let w = (12.0 + 7.5 * text.len() as f64) * s;
                let badge = Rect::new(r.x + r.w - w - 6.0 * s, r.y + r.h - 22.0 * s, w, 16.0 * s);
                quads.push((
                    None,
                    Quad::solid(badge, Color([0.0, 0.0, 0.0, 0.62]), 8.0 * scale, none, op),
                ));
                texts.push(TextDraw {
                    text,
                    family: th.font_family.clone(),
                    size: 11.0 * scale,
                    rect: badge,
                    align: Align::Center,
                    color: Color([1.0, 1.0, 1.0, op]),
                    clip: None,
                });
            }
            if g.label > 0.0 {
                let c = th.gallery_label_color;
                let size = th.gallery_label_size * scale;
                // Long names end in "…" (in the middle, keeping the
                // extension) rather than running past the tile.
                let name = self.items[i].name();
                let max = (r.w - 8.0 * scale as f64).max(1.0) as f32;
                let key = (name, max as u32, size as u32);
                let text = match (self.gallery.labels.get(&key), &mut self.renderer) {
                    (Some(t), _) => t.clone(),
                    (None, Some(rd)) => {
                        let t = crate::ui::shorten(
                            &mut rd.text,
                            &key.0,
                            &th.font_family,
                            size,
                            max,
                            false,
                        );
                        if self.gallery.labels.len() > 4000 {
                            self.gallery.labels.clear();
                        }
                        self.gallery.labels.insert(key, t.clone());
                        t
                    }
                    (None, None) => key.0,
                };
                texts.push(TextDraw {
                    text,
                    family: th.font_family.clone(),
                    size,
                    rect: Rect::new(r.x, r.y + r.h, r.w, g.label * zoom),
                    align: Align::Center,
                    color: c.with_alpha(c.0[3] * op),
                    clip: None,
                });
            }
        }
    }
}
