//! The winit event loop: window, input and timer events.

use super::*;

/// The binding supplies direction and sensitivity. Wheel units are rows,
/// precision input is already in physical pixels.
fn gallery_scroll_input(delta: MouseScrollDelta, rows: f64, row_height: f64) -> ScrollInput {
    match delta {
        MouseScrollDelta::LineDelta(_, lines) => {
            ScrollInput::Wheel(-rows * f64::from(lines.abs()) * row_height)
        }
        MouseScrollDelta::PixelDelta(p) => ScrollInput::Pixels(-rows * p.y.abs()),
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let transparent = self
            .script
            .shared
            .borrow()
            .settings
            .theme
            .window_transparent;
        // app_id, for compositor window rules (`class:wlens`); WLENS_APP_ID
        // gives a config its own, e.g. to float only the lightbox.
        let app_id = std::env::var("WLENS_APP_ID")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "wlens".into());
        let attrs = Window::default_attributes()
            .with_title("wlens")
            .with_transparent(transparent)
            .with_name(app_id.clone(), app_id);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                crate::io::notify::startup_failure("create window", &e);
                self.startup_failed = true;
                return event_loop.exit();
            }
        };
        if self.start_fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        self.scale = window.scale_factor() as f32;
        let renderer = match Renderer::new(&self.instance, window.clone(), transparent) {
            Ok(r) => r,
            Err(e) => {
                crate::io::notify::startup_failure("renderer", &e);
                self.startup_failed = true;
                return event_loop.exit();
            }
        };
        self.loader = Some(Loader::new(self.proxy.clone(), renderer.max_texture_dim()));
        self.renderer = Some(renderer);
        self.window = Some(window);

        self.layout_ui();
        self.update_browser_view();
        self.sync();
        self.script.emit("start");
        self.drain_queue(event_loop);
        if self.items.is_empty() {
            log::info!("no items yet; waiting for a script to add some");
        }
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Fire due Lua timers (`wlens.after`); a timer may schedule another.
        let due = self.script.due_timers(Instant::now());
        if !due.is_empty() {
            for f in due {
                self.script.call(&f);
            }
            self.drain_queue(event_loop);
            self.redraw();
        }
        if self.frame_retry.is_some_and(|t| t <= Instant::now()) {
            self.frame_retry = None;
            self.redraw();
        }
        let next = [
            self.frame_retry,
            self.idle_deadline(),
            self.script.next_timer(),
            self.anim_deadline(),
        ]
        .into_iter()
        .flatten()
        .min();
        event_loop.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::ArchiveListed { path, result } => {
                if self.listing_archive.as_ref() == Some(&path) {
                    self.listing_archive = None;
                    match result {
                        Ok(pages) => {
                            let title = self.items.get(self.index).map(Item::name);
                            self.open_archive_folder(
                                crate::model::archive_folder::ArchiveFolder::opened(&path, pages),
                                title,
                            );
                        }
                        Err(e) => self.toast(&e),
                    }
                }
            }
            UserEvent::Loaded { src, result } => self.on_loaded(src, result),
            UserEvent::Thumb { src, level, result } => self.on_thumb(src, level, result),
            #[cfg(feature = "video")]
            UserEvent::VideoCached { src, path } => self.on_video_cached(src, path),
            #[cfg(feature = "video")]
            UserEvent::Video { id, event } => self.on_video(event_loop, id, event),
            UserEvent::Download { id, progress, done } => {
                self.script.download_event(id, progress, done);
                self.flush_script();
                self.drain_queue(event_loop);
                // Callbacks usually change what UI text shows (progress).
                self.text_dirty = true;
                self.redraw();
            }
            UserEvent::Fetched { id, result } => self.script.fetched(id, result),
            UserEvent::Ipc { request, reply } => {
                let answer = self.on_ipc(event_loop, &request);
                let _ = reply.send(answer);
            }
        }
        self.drain_queue(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize((size.width, size.height));
                }
                self.layout_ui();
                self.refit_or_clamp();
                if let Some(c) = &mut self.current {
                    // Resizing shouldn't animate; the window is already moving.
                    c.view = self.target;
                }
                if self.gallery.active {
                    self.ensure_selection_visible();
                    self.gallery.shown = self.gallery.scroll;
                }
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.gallery.requested = None;
                self.redraw();
            }
            // Back on screen (another workspace, screen on again): if frames
            // failed while away, start over with a fresh surface right away.
            WindowEvent::Focused(true) | WindowEvent::Occluded(false) => {
                if let Some(r) = self.renderer.as_mut().filter(|r| r.stuck()) {
                    r.rebuild_surface();
                    self.frame_failures = 0;
                    self.frame_retry = None;
                }
                self.redraw();
            }
            WindowEvent::Focused(false) => {
                self.press = None;
                self.update_cursor();
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(m) => {
                if self.mods != m.state() {
                    self.scroll_accum = 0.0;
                    self.gallery.resize_anchor = None;
                }
                self.mods = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.focus.is_some() && self.input_key(event_loop, &event) {
                    return;
                }
                if let Some(chord) = Chord::from_key_event(&event, self.mods) {
                    log::trace!("key {chord:?}");
                    self.trigger(event_loop, &chord);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A scrollable UI area under the cursor (e.g. a long sidebar) takes it.
                if let Some(c) = self.cursor {
                    let input = match delta {
                        MouseScrollDelta::LineDelta(_, y) => {
                            ui::ScrollInput::Wheel(y as f64 * 48.0 * self.scale as f64)
                        }
                        MouseScrollDelta::PixelDelta(p) => ui::ScrollInput::Pixels(p.y),
                    };
                    if self.ui.scroll_at((c.x, c.y), input) {
                        self.redraw();
                        return;
                    }
                }
                // Continuous gallery bindings consume fractional input. Other
                // bindings (e.g. next image) still fire in discrete steps.
                const PX_PER_STEP: f64 = 40.0;
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / (PX_PER_STEP * self.scale as f64),
                };
                if self.gallery.active && steps != 0.0 {
                    let name = if steps > 0.0 {
                        "scrollup"
                    } else {
                        "scrolldown"
                    };
                    let chord = Chord::named(name, self.mods);
                    if let Some(Action::Command(Command::Gallery(action, factor))) =
                        self.script.binding(&chord, self.mode())
                    {
                        if action == "scroll" {
                            let grid = self.grid();
                            let input =
                                gallery_scroll_input(delta, factor.unwrap_or(1.0), grid.row_h());
                            self.scroll_gallery(grid, input);
                            return;
                        }
                        if matches!(action.as_str(), "bigger" | "smaller") {
                            let step = factor.unwrap_or(1.15).powf(steps.abs());
                            self.zoom_anchor = self.cursor.map(|c| (c.x, c.y));
                            self.exec(event_loop, Command::Gallery(action, Some(step)));
                            self.zoom_anchor = None;
                            return;
                        }
                    }
                }
                self.gallery.resize_anchor = None;
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_accum += p.y;
                        let steps = (self.scroll_accum / PX_PER_STEP).trunc();
                        self.scroll_accum -= steps * PX_PER_STEP;
                        steps
                    }
                };
                if dy != 0.0 {
                    let name = if dy > 0.0 { "scrollup" } else { "scrolldown" };
                    self.zoom_anchor = self.cursor.map(|c| (c.x, c.y));
                    self.trigger(event_loop, &Chord::named(name, self.mods));
                    self.zoom_anchor = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(i) = self
                    .press
                    .as_ref()
                    .filter(|p| p.slider)
                    .and_then(|p| p.target)
                {
                    self.press.as_mut().unwrap().dragged = true;
                    self.slide(event_loop, i, position.x);
                } else if let Some(press) = &mut self.press {
                    let moved = (position.x - press.at.x).hypot(position.y - press.at.y);
                    if press.blocks_drag {
                        // A press on the UI that moves is no longer a click,
                        // but it doesn't drag what's behind the UI either.
                        press.dragged |= moved > 4.0;
                    } else if press.dragged || moved > 4.0 {
                        let last = self.cursor.unwrap_or(press.at);
                        press.dragged = true;
                        let (dx, dy) = (position.x - last.x, position.y - last.y);
                        if self.gallery.active {
                            // Drag the grid like a touch list.
                            let g = self.grid();
                            let max = g.max_scroll(self.grid_len(&g));
                            self.gallery.scroll = (self.gallery.scroll - dy).clamp(0.0, max);
                            self.gallery.shown = self.gallery.scroll;
                        } else if press.button == MouseButton::Middle {
                            self.pan_to_pointer(position);
                        } else {
                            self.sync_pan_mode();
                            let (img, vp) = (self.img_size(), self.viewport);
                            self.target.pan(dx, dy, img, vp);
                            if let Some(c) = &mut self.current {
                                // Dragging must track the hand exactly; no easing.
                                c.view = self.target;
                            }
                            self.update_info();
                        }
                    }
                }
                self.cursor = Some(position);
                self.last_motion = Instant::now();
                let p = (position.x, position.y);
                let thumb = self.update_cursor();
                // Mouse motion alone changes nothing on screen unless it
                // drags, moves between hover targets, or reveals controls.
                let hover = (self.ui.hover_key(p), thumb);
                if self.press.as_ref().is_some_and(|p| p.dragged)
                    || hover != self.last_hover
                    || self.ui.needs_reveal()
                {
                    self.last_hover = hover;
                    self.redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.press = None;
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left | MouseButton::Middle, ElementState::Pressed) => {
                    if self.press.is_none()
                        && let Some(at) = self.cursor
                    {
                        let hit = self.ui.hit((at.x, at.y));
                        let slider = hit.clickable.filter(|&i| {
                            button == MouseButton::Left && self.ui.nodes[i].slider.is_some()
                        });
                        // A custom click binding gets a chance to run without moving the view.
                        let seek_on_press = button == MouseButton::Middle
                            && !hit.blocks_drag
                            && !self.gallery.active
                            && self
                                .script
                                .binding(&Chord::named("middleclick", self.mods), self.mode())
                                .is_none();
                        self.press = Some(Press {
                            button,
                            mods: self.mods,
                            at,
                            dragged: seek_on_press,
                            target: hit.clickable,
                            slider: slider.is_some(),
                            blocks_drag: hit.blocks_drag,
                        });
                        if let Some(i) = slider {
                            self.slide(event_loop, i, at.x);
                        } else if seek_on_press {
                            self.pan_to_pointer(at);
                            self.update_cursor();
                        }
                    }
                }
                (MouseButton::Left | MouseButton::Middle, ElementState::Released) => {
                    if let Some(p) = self.press.take_if(|p| p.button == button)
                        && !p.dragged
                        && !p.slider
                        && self.ui.hit((p.at.x, p.at.y)).clickable == p.target
                    {
                        if button == MouseButton::Middle {
                            self.trigger(event_loop, &Chord::named("middleclick", p.mods));
                        } else {
                            self.click(event_loop, p.at);
                        }
                    }
                    self.update_cursor();
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    self.trigger(event_loop, &Chord::named("rightclick", self.mods));
                }
                // The side buttons (browser-style back / forward).
                (MouseButton::Back, ElementState::Pressed) => {
                    self.trigger(event_loop, &Chord::named("mouseback", self.mods));
                }
                (MouseButton::Forward, ElementState::Pressed) => {
                    self.trigger(event_loop, &Chord::named("mouseforward", self.mods));
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_scroll_follows_the_displayed_grid() {
        let (mut shown, mut target) = (20.0, 20.0);
        let mut apply = |delta, rows| {
            gallery_scroll_input(delta, rows, 180.0).apply(&mut shown, &mut target, 500.0);
            (shown, target)
        };
        assert_eq!(
            apply(MouseScrollDelta::LineDelta(0.0, -1.5), 1.0),
            (20.0, 290.0)
        );
        let pixel = |dy| MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, dy));
        assert_eq!(apply(pixel(-0.25), 1.0), (20.25, 20.25));
        assert_eq!(apply(pixel(2.0), -2.0), (16.25, 16.25));
        assert_eq!(apply(pixel(100.0), -1.0), (0.0, 0.0));
    }
}
