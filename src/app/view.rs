//! Showing the current image: syncing with the cache, displaying, info for Lua, animation frames.

use super::*;

impl App {
    /// Show the current item if decoded, and (re)queue preloads around it.
    /// In gallery mode only thumbnails load; the full image waits for `open`.
    pub(super) fn sync(&mut self) {
        if self.listing_archive.is_some()
            && self.current_src().and_then(Source::local_path) != self.listing_archive.as_deref()
        {
            self.listing_archive = None;
        }
        let Some(src) = self.current_src().cloned() else {
            self.update_info();
            return;
        };
        if !self.gallery.active && self.items.get(self.index).is_some_and(|i| i.container) {
            // Containers aren't images: show them in the grid instead.
            self.gallery.active = true;
            self.ensure_selection_visible();
        }
        if self.gallery.active {
            #[cfg(feature = "video")]
            self.stop_video(); // nothing plays behind the grid
            self.update_info();
            return;
        }
        // Videos play through GStreamer instead of the image decoder.
        #[cfg(feature = "video")]
        {
            if self.items.get(self.index).is_some_and(|i| i.is_video()) {
                self.ensure_video(&src);
                self.update_info();
                return;
            }
            self.stop_video();
        }
        if self.current.as_ref().is_none_or(|l| l.src != src)
            && let Some(Slot::Ready(img)) = self.cache.get(&src)
        {
            let img = img.clone();
            self.display(src.clone(), &img);
        }

        let Some(loader) = &self.loader else { return };
        // Image jobs still queued but not started are discarded; forget their
        // Pending marks so they can be requested again.
        for job in loader.take_queued(|j| matches!(j, Job::Image(_))) {
            if let Job::Image(s) = job {
                self.cache.slots.remove(&s);
            }
        }
        let (preload, wrap) = {
            let s = &self.script.shared.borrow().settings;
            (s.preload as isize, s.wrap)
        };
        let n = self.items.len() as isize;
        let mut wanted = Vec::new();
        // Current first, then alternating ahead/behind.
        for d in std::iter::once(0).chain((1..=preload).flat_map(|d| [d, -d])) {
            let i = self.index as isize + d;
            let i = if wrap { i.rem_euclid(n) } else { i };
            if !(0..n).contains(&i) {
                continue;
            }
            // Containers aren't images; videos play through GStreamer.
            if self.items[i as usize].container || self.items[i as usize].is_video() {
                continue;
            }
            let s = &self.items[i as usize].src;
            if !self.cache.slots.contains_key(s) && !wanted.contains(s) {
                wanted.push(s.clone());
            }
        }
        for s in &wanted {
            self.cache.slots.insert(s.clone(), Slot::Pending);
        }
        loader.request_front(wanted.into_iter().map(Job::Image).collect());
        self.update_info();
    }

    pub(super) fn display(&mut self, src: Source, img: &Decoded) {
        let Some(r) = &mut self.renderer else { return };
        // Animations: one texture per frame; frame 0 is shown first.
        let frames: Vec<(TexId, u32)> = img
            .frames
            .iter()
            .map(|f| (r.upload(&f.rgba, img.size), f.delay_ms))
            .collect();
        let tex = match frames.first() {
            Some((t, _)) => *t,
            None => r.upload(&img.rgba, img.size),
        };
        let (kind, fit) = {
            let s = &self.script.shared.borrow().settings;
            (s.theme.transition, s.fit)
        };

        // Start a transition from whatever was showing.
        if let Some(old) = self.transition.take().and_then(|t| t.from) {
            old.release(r);
        }
        let prev = self.current.take();
        if kind == TransitionKind::None {
            if let Some(p) = prev {
                p.release(r);
            }
        } else {
            self.transition = Some(Transition {
                from: prev,
                start: Instant::now(),
                dir: self.nav_dir,
            });
        }

        self.layout_ui();
        self.target = View::default();
        self.sync_pan_mode();
        self.target
            .fit(fit, img.orig_size, self.viewport, self.margin());
        self.current = Some(Layer {
            src,
            tex,
            size: img.orig_size,
            file_size: img.file_size,
            view: self.target,
            frame_due: Instant::now()
                + Duration::from_millis(frames.first().map_or(0, |f| f.1 as u64)),
            frames,
            frame: 0,
        });
        self.update_info();
        self.script.emit("image");
        self.flush_script();
        self.redraw();
    }

    pub(super) fn update_info(&mut self) {
        let n = self.items.len();
        let wrap = self.script.shared.borrow().settings.wrap;
        let info = self.items.get(self.index).map(|item| {
            let shown = self.current.as_ref().filter(|l| l.src == item.src);
            let thumb = self.gallery.thumbs.get(&item.src).and_then(|t| t.orig);
            let (width, height) = shown.map(|l| l.size).or(thumb).unwrap_or((0, 0));
            let zoomed_in = shown.is_some_and(|l| {
                let (w, h) = self.target.rotated_size(l.size);
                w * self.target.zoom > self.viewport.w + 0.5
                    || h * self.target.zoom > self.viewport.h + 0.5
            });
            #[cfg(feature = "video")]
            let vinfo = self.video_info(&item.src);
            #[cfg(not(feature = "video"))]
            let vinfo: Option<(f64, f64, bool)> = None;
            let video = self.current_is_video() && !self.gallery.active;
            // On an item whose picture isn't on screen yet: still downloading
            // or decoding (an image), or fetching / starting (a video).
            #[cfg(feature = "video")]
            let video_waiting = self.video_pending.as_ref() == Some(&item.src)
                || self.video.as_ref().is_some_and(|v| v.src == item.src);
            #[cfg(not(feature = "video"))]
            let video_waiting = false;
            let loading = !self.gallery.active
                && !item.container
                && shown.is_none()
                && (matches!(self.cache.slots.get(&item.src), Some(Slot::Pending))
                    || video_waiting);
            // (`vinfo` is always None without the video feature.)
            #[cfg_attr(not(feature = "video"), allow(clippy::unnecessary_literal_unwrap))]
            let (position, duration, paused) = vinfo.unwrap_or((0.0, 0.0, false));
            let path = item.src.as_string();
            let dir = match &item.src {
                Source::Archive { archive, .. } => archive.to_string_lossy().into_owned(),
                Source::Path(p) => p
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                Source::Url(u) => u
                    .rsplit_once('/')
                    .map(|(d, _)| d.to_string())
                    .unwrap_or_default(),
            };
            ImageInfo {
                path,
                name: item.name(),
                dir,
                remote: item.src.is_remote(),
                index: self.index,
                count: n,
                width,
                height,
                zoom: self.target.zoom,
                fitted: self.target.fitted,
                zoomed_in,
                has_prev: n > 1 && (wrap || self.index > 0),
                has_next: n > 1 && (wrap || self.index + 1 < n),
                mode: self.mode(),
                container: item.container,
                item_count: item.count.clone(),
                meta: item.meta.clone(),
                loading,
                depth: self.stack.len(),
                level: self.script.shared.borrow().level.clone(),
                video,
                position,
                duration,
                paused,
                volume: self.volume,
                muted: self.muted,
                frames: shown.map_or(0, |l| l.frames.len()),
                frame: shown.map_or(0, |l| l.frame + 1),
                file_size: shown.map_or(0, |l| l.file_size),
            }
        });
        let mut sh = self.script.shared.borrow_mut();
        sh.info = info;
        sh.depth = self.stack.len();
        drop(sh);
        self.text_dirty = true;
    }

    pub(super) fn on_loaded(&mut self, src: Source, result: LoadResult) {
        match result {
            Ok(img) => {
                let budget = self.script.shared.borrow().settings.cache_mb * 1024 * 1024;
                let keep = self.current_src().cloned();
                self.cache.insert(src, img, budget, keep.as_ref());
                self.sync();
            }
            Err(e) => {
                // Drop unreadable items from the list rather than showing an error page.
                log::warn!("{}: {e}", src.as_string());
                self.cache.slots.remove(&src);
                self.remove_item(&src);
            }
        }
    }

    pub(super) fn remove_item(&mut self, src: &Source) {
        if let Some(i) = self.items.iter().position(|it| it.src == *src) {
            self.items.remove(i);
            if i < self.index || self.index >= self.items.len() {
                self.index = self.index.saturating_sub(1);
            }
            self.script.shared.borrow_mut().items = self.items.clone();
        }
        self.gallery.requested = None;
        self.sync();
    }

    pub(super) fn refit_or_clamp(&mut self) {
        self.sync_pan_mode();
        let (img, vp) = (self.img_size(), self.viewport);
        if self.target.fitted {
            let fit = self.script.shared.borrow().settings.fit;
            let margin = self.margin();
            self.target.fit(fit, img, vp, margin);
        } else {
            self.target.clamp(img, vp);
        }
        self.update_info();
    }

    /// Step an animated image to whichever frame is due now.
    pub(super) fn advance_animation(&mut self, now: Instant) {
        if self.gallery.active || self.anim_paused {
            return;
        }
        let Some(l) = &mut self.current else { return };
        if l.frames.len() < 2 || now < l.frame_due {
            return;
        }
        // Catch up without spinning if we were asleep for several frames.
        let mut guard = 0;
        while now >= l.frame_due && guard < l.frames.len() {
            l.frame = (l.frame + 1) % l.frames.len();
            l.frame_due += Duration::from_millis(l.frames[l.frame].1 as u64);
            guard += 1;
        }
        if now >= l.frame_due {
            l.frame_due = now + Duration::from_millis(l.frames[l.frame].1 as u64);
        }
        l.tex = l.frames[l.frame].0;
        // Just the frame number: a full update_info would re-run every UI
        // text function on each frame of a GIF.
        let frame = l.frame + 1;
        if let Some(i) = &mut self.script.shared.borrow_mut().info {
            i.frame = frame;
        }
    }

    /// When the next animation frame is due (to wake the event loop for it).
    pub(super) fn anim_deadline(&self) -> Option<Instant> {
        if self.gallery.active || self.anim_paused {
            return None;
        }
        self.current
            .as_ref()
            .filter(|l| l.frames.len() > 1)
            .map(|l| l.frame_due)
    }
}
