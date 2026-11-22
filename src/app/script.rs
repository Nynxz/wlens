//! Glue between the Lua host and the app: applying what scripts asked for, UI text and layout.

use super::*;

impl App {
    pub(super) fn sync_visibility(&self) {
        self.script.shared.borrow_mut().visible = self.ui.visibility();
    }

    /// Apply whatever Lua asked for during the last call.
    pub(super) fn flush_script(&mut self) {
        let (title, ui, items, fetches, settings_dirty) = {
            let mut sh = self.script.shared.borrow_mut();
            let fetches = if self.loader.is_some() {
                std::mem::take(&mut sh.fetch_queue)
            } else {
                Vec::new()
            };
            // Downloads: one thread each, reporting back through the event loop.
            for (id, url, dest) in std::mem::take(&mut sh.download_queue) {
                let proxy = self.proxy.clone();
                let p2 = proxy.clone();
                let spawned = std::thread::Builder::new()
                    .name("wlens-download".into())
                    .spawn(move || {
                        let res = crate::io::remote::download(&url, &dest, |bytes, total| {
                            let _ = p2.send_event(UserEvent::Download {
                                id,
                                progress: Some((bytes, total)),
                                done: None,
                            });
                        })
                        .map_err(|e| e.to_string());
                        let _ = proxy.send_event(UserEvent::Download {
                            id,
                            progress: None,
                            done: Some(res),
                        });
                    });
                if let Err(e) = spawned {
                    log::error!("download thread: {e}");
                }
            }
            // Options the user changed: save them for next time.
            if !sh.changed_options.is_empty() {
                let keys = std::mem::take(&mut sh.changed_options);
                for k in keys {
                    let v = match sh.settings.get(&k) {
                        Some(crate::model::theme::Value::Str(s)) => s,
                        Some(crate::model::theme::Value::Num(n)) => n.to_string(),
                        Some(crate::model::theme::Value::Bool(b)) => b.to_string(),
                        None => continue,
                    };
                    crate::io::state::put(
                        &sh.script_id,
                        "options",
                        &k,
                        Some(serde_json::Value::String(v)),
                    );
                }
            }
            let dirty = std::mem::take(&mut sh.settings_dirty);
            if std::mem::take(&mut sh.ui_dirty) {
                self.text_dirty = true;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            (
                sh.title.take(),
                sh.pending_ui.take(),
                std::mem::take(&mut sh.pending_items),
                fetches,
                dirty,
            )
        };
        // Script errors: show the first line of the latest one.
        let errors = std::mem::take(&mut self.script.shared.borrow_mut().errors);
        if let Some(last) = errors.last() {
            let line = last.lines().next().unwrap_or("").trim();
            let line = line.strip_prefix("runtime error: ").unwrap_or(line);
            // "/long/path/to/script.lua:12: msg" -> "script.lua:12: msg"
            let line = match line.find(".lua:") {
                Some(end) => match line[..end].rfind(['/', '.']) {
                    Some(cut) if line[..end].contains('/') => {
                        &line[line[..end].rfind('/').unwrap_or(cut) + 1..]
                    }
                    _ => line,
                },
                None => line,
            };
            let line: String = line.chars().take(140).collect();
            let more = if errors.len() > 1 {
                format!(" (+{} more)", errors.len() - 1)
            } else {
                String::new()
            };
            self.toast(&format!("Script error: {line}{more}"));
        }
        if settings_dirty {
            self.update_browser_view();
        }
        if settings_dirty && self.renderer.is_some() {
            // Lua changed settings directly: re-fit, re-query thumbnails, refresh text.
            self.gallery.requested = None;
            self.refit_or_clamp();
            self.redraw();
        }
        if let (Some(t), Some(w)) = (title, &self.window) {
            w.set_title(&t);
        }
        if let Some(ui) = ui {
            self.ui = ui;
            self.press = None;
            self.text_dirty = true;
            self.sync_visibility();
            self.redraw();
        }
        for op in items {
            self.apply_op(op);
        }
        if let Some(loader) = &self.loader
            && !fetches.is_empty()
        {
            loader.request_front(
                fetches
                    .into_iter()
                    .map(|(id, req)| Job::Fetch(id, req))
                    .collect(),
            );
        }
    }

    pub(super) fn drain_queue(&mut self, event_loop: &ActiveEventLoop) {
        loop {
            self.flush_script();
            let cmds = std::mem::take(&mut self.script.shared.borrow_mut().queue);
            if cmds.is_empty() {
                break;
            }
            for c in cmds {
                self.exec(event_loop, c);
            }
        }
    }

    /// Re-evaluate text, lay out the UI, and update the viewport.
    pub(super) fn layout_ui(&mut self) {
        if self.text_dirty {
            self.text_dirty = false;
            let script = &self.script;
            let info = script.shared.borrow().info.clone().unwrap_or_default();
            let info_table = script.info_value();
            let expand = |t: &str| {
                let sh = script.shared.borrow();
                ui::expand(t, &|k| info.field(k, &sh.settings))
            };
            // `children = function` elements rebuild when the item (or list)
            // changes, the window is resized (they may size to it), or script
            // code ran (they may read any script state).
            let (ww, wh, generation) = {
                let sh = script.shared.borrow();
                (sh.window.0, sh.window.1, sh.generation)
            };
            let key = format!(
                "{}|{}|{}|{}|{}|{}x{}|{generation}",
                info.path, info.index, info.count, info.mode, info.depth, ww as i64, wh as i64
            );
            if self.ui.update_dynamic(&info_table, &key) {
                // Node indices may have changed during tree compaction.
                self.press = None;
            }
            self.ui.refresh(&expand, &info_table);
            if self
                .focus
                .as_deref()
                .is_some_and(|id| !self.ui.input_visible(id))
            {
                self.focus = None;
            }
            let sh = self.script.shared.borrow();
            self.ui.apply_inputs(&sh.inputs, self.focus.as_deref());
            drop(sh);
            self.sync_visibility();
            // Text functions may have called wlens.* APIs.
            self.drain_title_only();
        }
        let Some(r) = &mut self.renderer else { return };
        let win = Rect::new(0.0, 0.0, r.size().0 as f64, r.size().1 as f64);
        let logical = (win.w / self.scale as f64, win.h / self.scale as f64);
        if self.script.shared.borrow().window != logical {
            // Lua layouts may depend on the window size (e.g. help columns).
            self.script.shared.borrow_mut().window = logical;
            self.text_dirty = true;
            return self.layout_ui();
        }
        let theme = self.script.shared.borrow().settings.theme.clone();
        self.ui.layout(win, self.scale, &theme, &mut r.text);
        // The image/grid area: the `view = "main"` element if the UI has one,
        // else the whole window; either way minus what floating elements reserve.
        // Space reserved only for the grid (floating controls) doesn't shrink
        // it: the grid scrolls under them, with that much extra padding so the
        // first and last rows can come fully into view.
        let [t, rt, b, l] = self.ui.reserved(win, false);
        let [gt, _, gb, _] = self.ui.reserved(win, self.gallery.active);
        let base = self.ui.view_rect("main").unwrap_or(win);
        let (x0, y0) = (base.x.max(l), base.y.max(t));
        let (x1, y1) = (
            (base.x + base.w).min(win.w - rt),
            (base.y + base.h).min(win.h - b),
        );
        let vp = Rect::new(x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0));
        let insets = (
            (gt - vp.y).max(0.0),
            ((vp.y + vp.h) - (win.h - gb)).max(0.0),
        );
        if insets != self.grid_insets {
            self.grid_insets = insets;
            self.gallery.requested = None;
        }
        self.scene_clip = self.ui.view_rect("main").map(|_| vp);
        let sc = self.scale as f64;
        let logical_vp = (vp.x / sc, vp.y / sc, vp.w / sc, vp.h / sc);
        if self.script.shared.borrow().view != logical_vp {
            // UI that sizes itself to the view (e.g. help columns) re-evaluates.
            self.script.shared.borrow_mut().view = logical_vp;
            self.text_dirty = true;
            self.redraw();
        }
        if vp != self.viewport {
            self.viewport = vp;
            self.gallery.requested = None;
            self.refit_or_clamp();
        }
    }

    pub(super) fn drain_title_only(&mut self) {
        let t = self.script.shared.borrow_mut().title.take();
        if let (Some(t), Some(w)) = (t, &self.window) {
            w.set_title(&t);
        }
    }
}
