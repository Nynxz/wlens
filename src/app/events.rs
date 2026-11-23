//! The winit event loop: window, input and timer events.

use super::*;

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
            UserEvent::Loaded { src, result } => self.on_loaded(src, result),
            UserEvent::Thumb { src, result } => self.on_thumb(src, result),
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
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
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
                    let px = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64 * 48.0 * self.scale as f64,
                        MouseScrollDelta::PixelDelta(p) => p.y,
                    };
                    if self.ui.scroll_at((c.x, c.y), px) {
                        self.redraw();
                        return;
                    }
                }
                // Touchpads send many small pixel deltas; turn them into
                // discrete steps so `ScrollUp` binds behave like a wheel.
                const PX_PER_STEP: f64 = 40.0;
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
                let ui_hit = self.ui.hit(p);
                let thumb = if self.gallery.active && !ui_hit.over_ui {
                    self.grid()
                        .index_at(p, self.gallery.shown, self.items.len())
                } else {
                    None
                };
                if let Some(w) = &self.window {
                    w.set_cursor(if ui_hit.pointer || thumb.is_some() {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    });
                }
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
                (MouseButton::Left, ElementState::Pressed) => {
                    if let Some(at) = self.cursor {
                        let hit = self.ui.hit((at.x, at.y));
                        let slider = hit.clickable.filter(|&i| self.ui.nodes[i].slider.is_some());
                        self.press = Some(Press {
                            at,
                            dragged: false,
                            target: hit.clickable,
                            slider: slider.is_some(),
                            blocks_drag: hit.blocks_drag,
                        });
                        if let Some(i) = slider {
                            self.slide(event_loop, i, at.x);
                        }
                    }
                }
                (MouseButton::Left, ElementState::Released) => {
                    if let Some(p) = self.press.take()
                        && !p.dragged
                        && !p.slider
                        && let Some(at) = self.cursor
                        && self.ui.hit((at.x, at.y)).clickable == p.target
                    {
                        self.click(event_loop, at);
                    }
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    self.trigger(event_loop, &Chord::named("rightclick", self.mods));
                }
                (MouseButton::Middle, ElementState::Pressed) => {
                    self.trigger(event_loop, &Chord::named("middleclick", self.mods));
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
