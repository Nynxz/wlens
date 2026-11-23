//! Per-frame work: advancing animations and assembling what the renderer draws.

use super::*;
use crate::model::settings::FitMode;

impl App {
    /// Advance animations by `dt`. Returns true while anything is moving.
    pub(super) fn step(&mut self, dt: f32) -> bool {
        let theme = self.script.shared.borrow().settings.theme.clone();
        let mut moving = false;
        if let Some(cur) = &mut self.current {
            let v = &mut cur.view;
            let t = &self.target;
            let eps = 1e-4 * t.zoom;
            let duration = if self.wheel_zooming {
                theme.anim_wheel_zoom
            } else {
                theme.anim_view
            };
            moving |= approach(&mut v.zoom, t.zoom, duration, dt, eps);
            moving |= approach(&mut v.offset.0, t.offset.0, duration, dt, 0.2);
            moving |= approach(&mut v.offset.1, t.offset.1, duration, dt, 0.2);
            v.quarter_turns = t.quarter_turns;
            v.flip_h = t.flip_h;
            v.flip_v = t.flip_v;
            v.fitted = t.fitted;
        }
        self.wheel_zooming &= moving;
        if let Some(tr) = &self.transition {
            if tr.start.elapsed().as_secs_f32() >= theme.transition_duration {
                if let (Some(from), Some(r)) = (
                    self.transition.take().and_then(|t| t.from),
                    &mut self.renderer,
                ) {
                    from.release(r);
                }
            } else {
                moving = true;
            }
        }
        let view_moving = moving;
        let g = &mut self.gallery;
        let target_t = g.active as u8 as f64;
        let gt = approach(&mut g.t, target_t, theme.gallery_anim, dt, 0.002);
        let gs = approach(&mut g.shown, g.scroll, theme.gallery_anim, dt, 0.3);

        let active = self.last_motion.elapsed().as_secs_f32() < theme.ui_idle;
        let cursor = self.cursor.map(|c| (c.x, c.y));
        let ui = self.ui.animate(dt, &theme, cursor, active);
        log::trace!("step: view/transition={view_moving} gallery_t={gt} scroll={gs} ui={ui}");
        moving || gt || gs || ui
    }

    pub(super) fn build_frame(&mut self) -> Frame {
        let s = self.script.shared.borrow().settings.clone();
        let th = &s.theme;
        let scale = self.scale;
        let vp = self.viewport;
        let mut quads = Vec::new();
        let mut texts = Vec::new();
        // Cross-fade between the image view and the grid.
        let gt = crate::geom::anim::Easing::EaseInOut.apply(self.gallery.t as f32);
        let image_op = 1.0 - gt;

        let draw_layer = |quads: &mut Vec<_>, l: &Layer, view: View, opacity: f32| {
            let opacity = opacity * image_op;
            if opacity <= 0.003 {
                return;
            }
            let rect = view.screen_rect(l.size, vp);
            let radius = th.image_radius * scale;
            if th.image_shadow.size > 0.0 {
                let sh = ui::scaled_shadow(th.image_shadow, scale);
                quads.push((None, Quad::shadow(rect, &sh, radius, opacity)));
            }
            let border = Border {
                width: th.image_border.width * scale,
                color: th.image_border.color,
            };
            let q = Quad::image(
                rect,
                view.screen_to_uv(l.size, vp),
                radius,
                border,
                th.checker,
                opacity,
            );
            quads.push((Some(l.tex), q));
        };

        let progress = self.transition.as_ref().map(|t| {
            let p = t.start.elapsed().as_secs_f32() / th.transition_duration.max(1e-3);
            th.transition_easing.apply(p)
        });
        match (&self.transition, progress, &self.current) {
            (Some(tr), Some(e), Some(cur)) => {
                let (mut from_v, mut to_v) = (tr.from.as_ref().map(|f| f.view), cur.view);
                let (mut from_op, mut to_op) = (1.0 - e, e);
                match th.transition {
                    TransitionKind::Slide => {
                        let dx = vp.w * tr.dir as f64;
                        if let Some(v) = &mut from_v {
                            v.offset.0 -= dx * e as f64;
                        }
                        to_v.offset.0 += dx * (1.0 - e) as f64;
                        (from_op, to_op) = (1.0, 1.0);
                        if tr.from.is_none() {
                            to_op = e;
                        }
                    }
                    TransitionKind::Zoom => {
                        if let Some(v) = &mut from_v {
                            v.zoom *= lerp(1.0, 1.06, e) as f64;
                        }
                        to_v.zoom *= lerp(0.92, 1.0, e) as f64;
                    }
                    TransitionKind::Fade | TransitionKind::None => {}
                }
                if let (Some(from), Some(v)) = (&tr.from, from_v) {
                    draw_layer(&mut quads, from, v, from_op);
                }
                draw_layer(&mut quads, cur, to_v, to_op);
            }
            (_, _, Some(cur)) => draw_layer(&mut quads, cur, cur.view, 1.0),
            _ => {}
        }

        if gt > 0.003 {
            self.draw_gallery(gt, &mut quads, &mut texts);
        }

        let ui_start = quads.len();
        let mut ui_quads = Vec::new();
        let mut ui_texts = Vec::new();
        let mut top_texts = Vec::new();
        let frame_no = self.gallery.frame;
        let thumbs = &mut self.gallery.thumbs;
        let mut lookup = |s: &str| {
            let t = thumbs.get_mut(&Source::parse(s))?;
            t.used = frame_no;
            Some((t.tex, t.size))
        };
        let top_start = ui_start
            + self.ui.draw(
                scale,
                th,
                &mut ui_quads,
                &mut ui_texts,
                &mut top_texts,
                &mut lookup,
            );
        let marks: HashMap<usize, TexId> = self.ui.tex_marks.iter().copied().collect();
        quads.extend(
            ui_quads
                .into_iter()
                .enumerate()
                .map(|(k, q)| (marks.get(&k).copied(), q)),
        );

        Frame {
            clear: th.window_background,
            checker: (th.checker_a, th.checker_b, th.checker_size * scale),
            filter: s.filter,
            quads,
            texts,
            ui_start,
            ui_texts,
            scene_clip: self.scene_clip,
            image_fit: (!self.gallery.active && self.current.is_some()).then_some(
                if self.target.fitted {
                    s.fit
                } else {
                    FitMode::None
                },
            ),
            bottom_anchored: self.renderer.as_ref().is_some_and(|r| {
                let height = f64::from(r.size().1);
                height - (vp.y + vp.h) > vp.y + 0.5
            }),
            top_start,
            top_texts,
        }
    }

    pub(super) fn render(&mut self, resized: bool) {
        let now = Instant::now();
        // After idling, the first frame shouldn't jump animations forward.
        let dt = if self.animating {
            now.duration_since(self.last_frame).as_secs_f32().min(0.1)
        } else {
            0.0
        };
        self.last_frame = now;
        self.advance_animation(now);
        // A click can change the target on the first frame after idling.
        self.refresh_ui();
        // Lay out after animating, so an element that just finished fading
        // out collapses (and frees its space) on this frame, not never.
        self.animating = self.step(dt) || dt == 0.0 && self.needs_motion();
        let old_grid = (resized && self.gallery.active).then(|| self.grid());
        self.layout_ui();
        if resized {
            // layout_ui has refitted the target to the new viewport. Present the
            // image and controls at the same geometry on the first resized frame.
            if let Some(layer) = &mut self.current {
                layer.view = self.target;
            }
            if let Some(old) = old_grid {
                let new = self.grid();
                let max = new.max_scroll(self.grid_len(&new));
                let gallery = &mut self.gallery;
                let shown = old.reflow_scroll(
                    &new,
                    self.items.len(),
                    gallery.shown,
                    &mut gallery.reflow_anchor,
                );
                gallery.scroll = (shown + (gallery.scroll - gallery.shown)).clamp(0.0, max);
                gallery.shown = shown.clamp(0.0, max);
            }
        }
        self.request_thumbs();
        self.request_ui_images();
        let frame = self.build_frame();
        let drawn = self.renderer.as_mut().is_none_or(|r| r.render(&frame));
        if drawn {
            self.frame_failures = 0;
            self.frame_retry = None;
            if self.animating {
                self.redraw();
            }
        } else {
            // Try again later rather than straight away: 100ms, doubling to 2s.
            self.frame_failures = (self.frame_failures + 1).min(6);
            let wait = (50u64 << self.frame_failures).min(2000);
            self.frame_retry = Some(now + std::time::Duration::from_millis(wait));
        }
    }

    /// Something will move as soon as time advances (first frame after idle).
    pub(super) fn needs_motion(&self) -> bool {
        let g = &self.gallery;
        self.transition.is_some()
            || self.current.as_ref().is_some_and(|c| c.view != self.target)
            || (g.t - g.active as u8 as f64).abs() > 0.002
            || (g.shown - g.scroll).abs() > 0.3
            // The grid's spinner while a list loads.
            || (g.active && self.items.is_empty() && self.script.shared.borrow().more)
    }

    /// When hover-revealed UI is showing, wake up to fade it out on time.
    pub(super) fn idle_deadline(&self) -> Option<Instant> {
        if !self.ui.has_hover_ui() {
            return None;
        }
        let idle = self.script.shared.borrow().settings.theme.ui_idle;
        let at = self.last_motion + Duration::from_secs_f32(idle) + Duration::from_millis(10);
        (at > Instant::now()).then_some(at)
    }
}
