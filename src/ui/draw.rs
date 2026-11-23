//! Turning laid-out nodes into quads and text (base layer, then the top
//! layer and the tooltip).

use super::*;

impl Ui {
    pub(super) fn zero_opacity(&mut self, i: usize) {
        self.nodes[i].opacity = 0.0;
        for c in self.nodes[i].children.clone() {
            self.zero_opacity(c);
        }
    }

    /// Draw the UI: base elements into `quads`/`texts`, then top-layer
    /// elements and the tooltip after them (their text into `top_texts`).
    /// Returns the index in `quads` where the top layer starts.
    pub fn draw(
        &mut self,
        scale: f32,
        theme: &Theme,
        quads: &mut Vec<Quad>,
        texts: &mut Vec<TextDraw>,
        top_texts: &mut Vec<TextDraw>,
        images: &mut ImageLookup,
    ) -> usize {
        self.tex_marks.clear();
        self.deferred.clear();
        self.drawing_top = false;
        for r in self.roots.clone() {
            self.draw_node(r, 1.0, scale, theme, quads, texts, images);
        }
        let top_start = quads.len();
        self.drawing_top = true;
        for (i, op) in std::mem::take(&mut self.deferred) {
            self.draw_node(i, op, scale, theme, quads, top_texts, images);
        }
        self.drawing_top = false;
        let texts = top_texts;
        if let Some((i, _)) = self.tip
            && self.tip_vis > 0.003
        {
            let op = self.tip_vis as f32;
            let r = self.tip_rect;
            let none = Border {
                width: 0.0,
                color: Color::TRANSPARENT,
            };
            let sh = Shadow {
                size: 8.0 * scale,
                color: Color([0.0, 0.0, 0.0, 0.4]),
                x: 0.0,
                y: 2.0 * scale,
            };
            quads.push(Quad::shadow(r, &sh, theme.tooltip_radius * scale, op));
            quads.push(Quad::solid(
                r,
                theme.tooltip_background,
                theme.tooltip_radius * scale,
                none,
                op,
            ));
            let c = theme.tooltip_color;
            texts.push(TextDraw {
                text: self.nodes[i].tooltip_value.clone(),
                family: theme.font_family.clone(),
                size: theme.tooltip_size * scale,
                rect: r,
                align: Align::Center,
                center_ink: false,
                color: c.with_alpha(c.0[3] * op),
                clip: None,
            });
        }
        top_start
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_node(
        &mut self,
        i: usize,
        parent_op: f32,
        scale: f32,
        theme: &Theme,
        quads: &mut Vec<Quad>,
        texts: &mut Vec<TextDraw>,
        images: &mut ImageLookup,
    ) {
        if self.nodes[i].layer_top && !self.drawing_top {
            self.deferred.push((i, parent_op));
            return;
        }
        let n = &self.nodes[i];
        let mut style = match &n.hover {
            Some(h) => n.style.lerp(h, n.hover_t as f32),
            None => n.style.clone(),
        };
        if let Some(s) = &n.selected {
            style = style.lerp(&s.style, s.progress as f32);
        }
        let op = parent_op * n.vis as f32 * style.opacity;
        self.nodes[i].opacity = op;
        if op <= 0.003 {
            // Nothing below is visible either (spinners stop animating,
            // hidden things aren't hovered).
            self.zero_opacity(i);
            return;
        }
        // Scrolled out of view entirely: nothing to draw (or click).
        let (rect, clip) = (self.nodes[i].rect, self.nodes[i].clip);
        if let Some(c) = clip
            && (rect.y > c.y + c.h || rect.y + rect.h < c.y)
        {
            self.zero_opacity(i);
            return;
        }
        let quads_before = quads.len();
        let texts_before = texts.len();
        let n = &self.nodes[i];
        let r = n.rect;
        if style.shadow.size > 0.0 && style.shadow.color.0[3] > 0.0 {
            quads.push(Quad::shadow(
                r,
                &scaled_shadow(style.shadow, scale),
                style.radius * scale,
                op,
            ));
        }
        if style.background.0[3] > 0.0 || style.border.width > 0.0 {
            let border = Border {
                width: style.border.width * scale,
                color: style.border.color,
            };
            quads.push(Quad::solid(
                r,
                style.background,
                style.radius * scale,
                border,
                op,
            ));
        }
        if !n.image_value.is_empty()
            && let Some((tex, size)) = images(&n.image_value)
        {
            let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let inner = Rect::new(
                r.x + pl,
                r.y + pt,
                (r.w - pl - pr).max(0.0),
                (r.h - pt - pb).max(0.0),
            );
            let (sx, sy) = (inner.w / size.0 as f64, inner.h / size.1 as f64);
            let v = crate::geom::view::View {
                zoom: if n.image_contain {
                    sx.min(sy)
                } else {
                    sx.max(sy)
                },
                ..Default::default()
            };
            let rect = if n.image_contain {
                v.screen_rect(size, inner)
            } else {
                inner
            };
            let rad = (style.radius * scale - pl.min(pt) as f32).max(0.0);
            let none = Border {
                width: 0.0,
                color: Color::TRANSPARENT,
            };
            quads.push(Quad::image(
                rect,
                v.screen_to_uv(size, inner),
                rad,
                none,
                false,
                op,
            ));
            self.tex_marks.push((quads.len() - 1, tex));
        }
        let n = &self.nodes[i];
        if let Some(rg) = &n.ring {
            let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let inner = Rect::new(
                r.x + pl,
                r.y + pt,
                (r.w - pl - pr).max(0.0),
                (r.h - pt - pb).max(0.0),
            );
            let d = inner.w.min(inner.h);
            let sq = Rect::new(
                inner.x + (inner.w - d) / 2.0,
                inner.y + (inner.h - d) / 2.0,
                d,
                d,
            );
            let (len, start) = if rg.spin {
                (rg.now.max(0.15), spin_phase())
            } else {
                (rg.now, 0.0)
            };
            quads.push(Quad::ring(
                sq,
                rg.thickness * scale,
                len,
                start,
                rg.fill,
                rg.track,
                op,
            ));
        }
        if let Some(s) = &n.slider {
            // The track is a pill centred in the padded box; padding widens
            // the clickable area without thickening the bar.
            let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let inner = Rect::new(
                r.x + pl,
                r.y + pt,
                (r.w - pl - pr).max(0.0),
                (r.h - pt - pb).max(0.0),
            );
            let rad = (inner.h / 2.0) as f32;
            let none = Border {
                width: 0.0,
                color: Color::TRANSPARENT,
            };
            quads.push(Quad::solid(inner, s.track, rad, none, op));
            if s.now > 0.0 {
                let w = (inner.w * s.now as f64).max(inner.h);
                quads.push(Quad::solid(
                    Rect::new(inner.x, inner.y, w, inner.h),
                    s.fill,
                    rad,
                    none,
                    op,
                ));
            }
            // Knob grows in on hover.
            let k = inner.h * (1.0 + 1.6 * n.hover_t);
            if n.hover_t > 0.01 {
                let cx = inner.x + inner.w * s.now as f64;
                let cy = inner.y + inner.h / 2.0;
                quads.push(Quad::solid(
                    Rect::new(cx - k / 2.0, cy - k / 2.0, k, k),
                    s.fill,
                    (k / 2.0) as f32,
                    none,
                    op,
                ));
            }
        }
        if n.input.is_some() && n.focused {
            // Caret after the text.
            let [pt, _, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let h = (n.font_size.unwrap_or(theme.font_size) * scale * 1.2) as f64;
            let cy = r.y + pt + (r.h - pt - pb - h) / 2.0;
            let color = style.color.unwrap_or(theme.font_color);
            let none = Border {
                width: 0.0,
                color: Color::TRANSPARENT,
            };
            let x = r.x + pl + n.caret as f64;
            quads.push(Quad::solid(
                Rect::new(x, cy, (1.5 * scale) as f64, h),
                color,
                0.0,
                none,
                op,
            ));
        }
        if (n.text.is_some() || n.input.is_some()) && !n.text_value.is_empty() {
            let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let inner = Rect::new(r.x + pl, r.y + pt, r.w - pl - pr, r.h - pt - pb);
            let mut color = style.color.unwrap_or(theme.font_color);
            if n.input_placeholder {
                color = color.with_alpha(color.0[3] * 0.45);
            }
            texts.push(TextDraw {
                text: n.display_text.clone(),
                family: n.family(theme),
                size: n.font_size.unwrap_or(theme.font_size) * scale,
                rect: inner,
                center_ink: n.on_click.is_some()
                    && n.input.is_none()
                    && n.text_align == Align::Center,
                align: if n.input.is_some() {
                    Align::Start
                } else {
                    n.text_align
                },
                color: color.with_alpha(color.0[3] * op),
                clip: None,
            });
        }
        // Everything this element drew stays inside its clip.
        let clip = self.nodes[i].clip;
        if clip.is_some() {
            for q in &mut quads[quads_before..] {
                *q = q.clipped(clip);
            }
            for t in &mut texts[texts_before..] {
                t.clip = clip;
            }
        }
        for c in self.nodes[i].children.clone() {
            self.draw_node(c, op, scale, theme, quads, texts, images);
        }
    }
}

pub fn scaled_shadow(s: Shadow, scale: f32) -> Shadow {
    Shadow {
        size: s.size * scale,
        color: s.color,
        x: s.x * scale,
        y: s.y * scale,
    }
}
