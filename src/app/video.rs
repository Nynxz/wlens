//! Video items: starting/stopping players, streaming frames into a layer,
//! and the `video` command. Built with the `video` feature.

use super::*;
use crate::io::video::{Player, VideoEvent};

pub(super) struct VideoState {
    pub(super) player: Player,
    pub(super) src: Source,
    pub(super) id: u64,
    /// Throttle for refreshing position/duration in UI text.
    pub(super) info_at: Instant,
    pub(super) ended: bool,
}

/// Videos up to this size are fetched into the cache before playing.
const VIDEO_CACHE_MAX: u64 = 128 * 1024 * 1024;

/// `file://` for local paths, the URL itself for remote ones (streamed).
fn uri_for(src: &Source) -> Option<String> {
    match src {
        Source::Url(u) => Some(u.clone()),
        Source::Path(p) => {
            let abs = std::fs::canonicalize(p).ok()?;
            gstreamer::glib::filename_to_uri(&abs, None)
                .ok()
                .map(|u| u.to_string())
        }
    }
}

impl App {
    /// Start playing `src` unless it's already the active video.
    /// Play `src` unless it already is. URLs are fetched into the download
    /// cache first (small videos), so replays and seeking don't hit the
    /// server again; big ones stream.
    pub(super) fn ensure_video(&mut self, src: &Source) {
        if self.video.as_ref().is_some_and(|v| &v.src == src)
            || self.video_pending.as_ref() == Some(src)
        {
            return;
        }
        self.stop_video();
        if let Source::Url(u) = src {
            match crate::io::remote::cached_path(u).filter(|p| p.exists()) {
                Some(p) => return self.start_player(src, Some(&p)),
                None => {
                    self.video_pending = Some(src.clone());
                    self.toast("Loading video…");
                    let (proxy, url, s) = (self.proxy.clone(), u.clone(), src.clone());
                    let _ = std::thread::Builder::new()
                        .name("wlens-video-fetch".into())
                        .spawn(move || {
                            let path = match crate::io::remote::cache_video(&url, VIDEO_CACHE_MAX) {
                                Ok(p) => p,
                                Err(e) => {
                                    log::warn!("video {url}: {e:#} — streaming instead");
                                    None
                                }
                            };
                            let _ = proxy.send_event(UserEvent::VideoCached { src: s, path });
                        });
                    return;
                }
            }
        }
        self.start_player(src, None);
    }

    /// The background fetch finished: play the cached file, or stream.
    pub(super) fn on_video_cached(&mut self, src: Source, path: Option<std::path::PathBuf>) {
        if self.video_pending.as_ref() != Some(&src) {
            return;
        }
        self.video_pending = None;
        // Still on that item (and not in the grid)?
        if self.gallery.active || self.current_src() != Some(&src) {
            return;
        }
        self.start_player(&src, path.as_deref());
        self.redraw();
    }

    fn start_player(&mut self, src: &Source, local: Option<&std::path::Path>) {
        let uri = match local {
            Some(p) => gstreamer::glib::filename_to_uri(p, None)
                .ok()
                .map(|u| u.to_string()),
            None => uri_for(src),
        };
        let Some(uri) = uri else {
            return self.toast("Can't open video");
        };
        self.next_video_id += 1;
        let id = self.next_video_id;
        match Player::new(&uri, id, self.proxy.clone(), self.volume, self.muted) {
            Ok(player) => {
                self.video = Some(VideoState {
                    player,
                    src: src.clone(),
                    id,
                    info_at: Instant::now(),
                    ended: false,
                });
            }
            Err(e) => {
                log::warn!("{}: {e:#}", src.as_string());
                self.toast("Can't play this video (missing GStreamer plugins?)");
            }
        }
    }

    pub(super) fn stop_video(&mut self) {
        self.video_pending = None;
        // Dropping the player stops its pipeline; its frames stay on screen
        // until something else is shown.
        self.video = None;
    }

    pub(super) fn on_video(&mut self, event_loop: &ActiveEventLoop, id: u64, event: VideoEvent) {
        let Some(v) = self.video.as_mut().filter(|v| v.id == id) else {
            return; // a player we already stopped
        };
        match event {
            VideoEvent::Frame => {
                let Some(frame) = v.player.take_frame() else {
                    return;
                };
                let src = v.src.clone();
                let Some(r) = &mut self.renderer else { return };
                let showing = self.current.as_ref().is_some_and(|l| l.src == src);
                if showing {
                    let l = self.current.as_mut().unwrap();
                    l.tex = r.stream(Some(l.tex), &frame.rgba, frame.size);
                    l.size = frame.size;
                } else {
                    // First frame: becomes the current layer, fitted like an image.
                    let tex = r.stream(None, &frame.rgba, frame.size);
                    if let Some(old) = self.transition.take().and_then(|t| t.from) {
                        old.release(r);
                    }
                    if let Some(prev) = self.current.take() {
                        prev.release(r);
                    }
                    self.layout_ui();
                    self.target = View::default();
                    self.sync_pan_mode();
                    let fit = self.script.shared.borrow().settings.fit;
                    self.target
                        .fit(fit, frame.size, self.viewport, self.margin());
                    let file_size = src
                        .local_path()
                        .and_then(|p| std::fs::metadata(p).ok())
                        .map_or(0, |m| m.len());
                    self.current = Some(Layer {
                        src,
                        tex,
                        size: frame.size,
                        file_size,
                        view: self.target,
                        frames: Vec::new(),
                        frame: 0,
                        frame_due: Instant::now(),
                    });
                    self.update_info();
                    self.script.emit("image");
                    self.flush_script();
                }
                // Position text: ~10 times a second is plenty.
                if let Some(v) = &mut self.video
                    && v.info_at.elapsed() > Duration::from_millis(100)
                {
                    v.info_at = Instant::now();
                    self.update_info();
                }
                self.redraw();
            }
            VideoEvent::Eos => {
                let end = self.script.shared.borrow().settings.video_end.clone();
                match end.as_str() {
                    "loop" => v.player.seek(0.0),
                    "next" if self.items.len() > 1 => {
                        v.ended = true;
                        self.exec(event_loop, Command::Next(1));
                        self.drain_queue(event_loop);
                    }
                    _ => {
                        v.ended = true;
                        v.player.set_paused(true);
                        self.update_info();
                    }
                }
            }
            VideoEvent::Error(e) => {
                log::warn!("video: {e}");
                self.toast(&format!("Video error: {e}"));
                self.stop_video();
            }
        }
    }

    pub(super) fn video_command(&mut self, action: &str, arg: Option<&str>) {
        let Some(v) = &mut self.video else { return };
        match action {
            "play" => v.player.set_paused(false),
            "pause" => v.player.set_paused(true),
            "toggle" => {
                if v.ended {
                    v.ended = false;
                    v.player.seek(0.0);
                }
                let p = !v.player.paused();
                v.player.set_paused(p);
            }
            "seek" => {
                let Some(a) = arg else { return };
                let pos = v.player.position().unwrap_or(0.0);
                let dur = v.player.duration().unwrap_or(0.0);
                let target = if let Some(pct) = a.strip_suffix('%') {
                    pct.parse::<f64>().ok().map(|p| dur * p / 100.0)
                } else if a.starts_with('+') || a.starts_with('-') {
                    a.parse::<f64>().ok().map(|d| pos + d)
                } else {
                    a.parse::<f64>().ok()
                };
                if let Some(t) = target {
                    let t = if dur > 0.0 {
                        t.clamp(0.0, (dur - 0.05).max(0.0))
                    } else {
                        t.max(0.0)
                    };
                    v.ended = false;
                    v.player.seek(t);
                }
            }
            "volume" => {
                let Some(a) = arg else { return };
                let Ok(n) = a.parse::<f64>() else { return };
                let new = if a.starts_with('+') || a.starts_with('-') {
                    self.volume * 100.0 + n
                } else {
                    n
                };
                self.volume = (new / 100.0).clamp(0.0, 1.5);
                self.muted = false;
                v.player.set_volume(self.volume, self.muted);
                self.toast(&format!("Volume {:.0}%", self.volume * 100.0));
            }
            "mute" => {
                self.muted = !self.muted;
                v.player.set_volume(self.volume, self.muted);
                self.toast(if self.muted { "Muted" } else { "Sound on" });
            }
            _ => {}
        }
        if matches!(action, "volume" | "mute") {
            // Remember for next time (all scripts share it).
            crate::io::state::put(
                "_global",
                "player",
                "volume",
                Some(serde_json::json!(self.volume)),
            );
            crate::io::state::put(
                "_global",
                "player",
                "muted",
                Some(serde_json::json!(self.muted)),
            );
        }
        self.update_info();
    }

    /// Playback state for `wlens.current()`: (position, duration, paused).
    pub(super) fn video_info(&self, src: &Source) -> Option<(f64, f64, bool)> {
        let v = self.video.as_ref().filter(|v| &v.src == src)?;
        Some((
            v.player.position().unwrap_or(0.0),
            v.player.duration().unwrap_or(0.0),
            v.player.paused() || v.ended,
        ))
    }
}
