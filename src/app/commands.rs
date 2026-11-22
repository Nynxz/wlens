//! Executing commands and dispatching keybindings.

use super::*;

impl App {
    pub fn exec(&mut self, event_loop: &ActiveEventLoop, cmd: Command) {
        log::debug!("exec {cmd:?}");
        self.sync_pan_mode();
        let (img, vp) = (self.img_size(), self.viewport);
        match cmd {
            Command::Next(d) => self.navigate_dir(self.index as isize + d, d.signum()),
            Command::First => self.navigate_dir(0, 1),
            Command::Last => self.navigate_dir(self.items.len() as isize - 1, -1),
            Command::Goto(i) => self.navigate_dir(i as isize, 1),
            Command::Zoom(z) => {
                let step = self.script.shared.borrow().settings.zoom_step;
                let new = match z {
                    Zoom::In => self.target.zoom * step,
                    Zoom::Out => self.target.zoom / step,
                    Zoom::To(z) => z,
                };
                let anchor = self.zoom_anchor.unwrap_or(vp.center());
                self.target.zoom_at(new, anchor, img, vp);
            }
            Command::Fit(mode) => {
                if let Some(m) = mode {
                    self.script.shared.borrow_mut().settings.fit = m;
                }
                self.target.fitted = true;
                self.refit_or_clamp();
            }
            Command::Pan(dx, dy) => {
                self.target
                    .pan(dx * self.scale as f64, dy * self.scale as f64, img, vp)
            }
            Command::Rotate(q) => {
                self.target.rotate(q);
                self.refit_or_clamp();
            }
            Command::FlipH => self.target.flip_h ^= true,
            Command::FlipV => self.target.flip_v ^= true,
            Command::Reset => {
                self.target = View::default();
                self.sync_pan_mode();
                self.refit_or_clamp();
            }
            Command::Set(k, v) => {
                let mut sh = self.script.shared.borrow_mut();
                match sh.settings.set(&k, &v) {
                    Ok(()) => sh.changed_options.push(k.clone()), // remembered next run
                    Err(e) => log::error!("set: {e}"),
                }
                drop(sh);
                // UI showing options (the settings dialog) re-reads them.
                self.text_dirty = true;
                self.redraw();
                if matches!(k.as_str(), "fit" | "image.margin" | "pan") {
                    self.refit_or_clamp();
                }
                if k.starts_with("gallery.") {
                    self.gallery.requested = None;
                }
                if k == "wrap" {
                    let on = self.script.shared.borrow().settings.wrap;
                    self.toast(if on {
                        "Wrap around: on"
                    } else {
                        "Wrap around: off"
                    });
                }
            }
            Command::Fullscreen(want) => {
                if let Some(w) = &self.window {
                    let on = want.unwrap_or(w.fullscreen().is_none());
                    w.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                }
            }
            Command::Reload => {
                if let Some(s) = self.current_src().cloned() {
                    self.cache.remove(&s);
                    self.cache.slots.remove(&s);
                    if let (Some(l), Some(r)) = (self.current.take(), &mut self.renderer) {
                        l.release(r);
                    }
                    self.sync();
                }
            }
            Command::Quit => event_loop.exit(),
            Command::Lua(code) => {
                if let Err(e) = self.script.run_str("=command", &code) {
                    log::error!("lua: {e}");
                }
            }
            Command::Ui(how, id) if how == "focus" => {
                if self.ui.input_actions(&id).is_none() {
                    log::warn!("ui focus: no input element with id '{id}'");
                    return;
                }
                self.focus = Some(id);
                self.text_dirty = true;
                self.redraw();
            }
            Command::Ui(how, _) if how == "blur" => {
                self.focus = None;
                self.text_dirty = true;
                self.redraw();
            }
            Command::Ui(how, id) => {
                if let Err(e) = self.ui.set_forced(&id, &how) {
                    log::error!("{e}");
                }
                self.sync_visibility();
            }
            Command::Copy(what) => self.copy(&what),
            Command::Gallery(action, rows) if action == "scroll" => {
                let grid = self.grid();
                self.scroll_gallery(
                    grid,
                    ScrollInput::Wheel(-rows.unwrap_or(1.0) * grid.row_h()),
                );
                return;
            }
            Command::Gallery(action, amount) => self.gallery_command(&action, amount),
            Command::Back => {
                // One step at a time: image view -> this level's grid -> parent level.
                if !self.gallery.active
                    && (!self.stack.is_empty() || self.items.iter().any(|i| i.container))
                {
                    self.set_gallery(true);
                } else if !self.back() {
                    self.toast("Already at the top");
                }
            }
            Command::Open => self.pending_open = true,
            Command::Video(action, arg) => {
                #[cfg(feature = "video")]
                self.video_command(&action, arg.as_deref());
                #[cfg(not(feature = "video"))]
                {
                    let _ = (action, arg);
                    self.toast("This build has no video support");
                }
            }
            Command::Animation(how) => {
                self.anim_paused = match how.as_str() {
                    "play" => false,
                    "pause" => true,
                    _ => !self.anim_paused,
                };
                if let Some(l) = &mut self.current {
                    // Resume from the current frame without skipping ahead.
                    l.frame_due = Instant::now()
                        + Duration::from_millis(l.frames.get(l.frame).map_or(0, |f| f.1 as u64));
                }
                if self.current.as_ref().is_some_and(|l| !l.frames.is_empty()) {
                    self.toast(if self.anim_paused {
                        "Paused"
                    } else {
                        "Playing"
                    });
                }
            }
            Command::Refresh => {
                if self.script.has_hooks("refresh") {
                    self.script.emit("refresh");
                } else {
                    return self.exec(event_loop, Command::Reload);
                }
            }
        }
        self.update_info();
        self.redraw();
        if std::mem::take(&mut self.pending_open) {
            self.open_current(event_loop);
        }
    }

    /// Copy via wl-copy (it forks to serve the selection, so we needn't stay alive).
    pub(super) fn copy(&self, what: &str) {
        let Some(item) = self.items.get(self.index) else {
            return;
        };
        let mut cmd = std::process::Command::new("wl-copy");
        cmd.stdin(std::process::Stdio::piped());
        let local = match &item.src {
            Source::Path(p) => Some(p.clone()),
            Source::Url(u) => crate::io::remote::cached_path(u).filter(|p| p.exists()),
        };
        let (data, label): (Vec<u8>, &str) = match what {
            "image" => {
                let Some(file) = local else {
                    return self.toast("Not downloaded yet");
                };
                let fmt = image::ImageFormat::from_path(&file).ok().or_else(|| {
                    std::fs::read(&file)
                        .ok()
                        .and_then(|b| image::guess_format(&b).ok())
                });
                if let Some(f) = fmt {
                    cmd.arg("--type").arg(f.to_mime_type());
                }
                match std::fs::read(&file) {
                    Ok(b) => (b, "Copied image"),
                    Err(e) => return log::error!("copy: {e}"),
                }
            }
            "name" => (item.name().into_bytes(), "Copied name"),
            "dir" => {
                let dir = match &item.src {
                    Source::Path(p) => std::fs::canonicalize(p)
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned())),
                    Source::Url(u) => u.rsplit_once('/').map(|(d, _)| d.to_string()),
                };
                (dir.unwrap_or_default().into_bytes(), "Copied folder")
            }
            _ => {
                let text = match &item.src {
                    Source::Path(p) => std::fs::canonicalize(p)
                        .unwrap_or(p.clone())
                        .to_string_lossy()
                        .into_owned(),
                    Source::Url(u) => u.clone(),
                };
                (
                    text.into_bytes(),
                    if item.src.is_remote() {
                        "Copied URL"
                    } else {
                        "Copied path"
                    },
                )
            }
        };
        let result = cmd.spawn().and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().expect("piped").write_all(&data)
        });
        match result {
            Ok(()) => self.toast(label),
            Err(e) => {
                log::error!("copy: wl-copy failed: {e}");
                self.toast("Copy failed: is wl-clipboard installed?");
            }
        }
    }

    pub(super) fn run_action(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        match action {
            Action::Command(c) => self.exec(event_loop, c),
            Action::Lua(f) => self.script.call(&f),
        }
        self.drain_queue(event_loop);
    }

    pub(super) fn trigger(&mut self, event_loop: &ActiveEventLoop, chord: &Chord) -> bool {
        let Some(action) = self.script.binding(chord, self.mode()) else {
            return false;
        };
        self.run_action(event_loop, action);
        true
    }
}
