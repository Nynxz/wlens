//! Video playback through GStreamer (the `video` cargo feature).
//!
//! `playbin` does demuxing, decoding, audio output and the clock; its video
//! goes to an `appsink` as RGBA frames, which the app uploads and draws like
//! any image — so zoom, pan, themes and overlays work on video too.

use std::cell::Cell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

const VIDEO_EXT: &[&str] = &[
    "webm", "mp4", "m4v", "mkv", "mov", "avi", "ogv", "mpg", "mpeg", "wmv", "3gp",
];

/// Is this a video, judging by its extension (a file name or URL path)?
pub fn is_video_name(name: &str) -> bool {
    let name = name.split(['?', '#']).next().unwrap_or(name);
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| VIDEO_EXT.contains(&ext.to_ascii_lowercase().as_str()))
}

fn init() -> anyhow::Result<()> {
    static INIT: Once = Once::new();
    static mut FAILED: bool = false;
    INIT.call_once(|| {
        if let Err(e) = gst::init() {
            log::error!("GStreamer: {e}");
            // SAFETY: written once inside call_once, read after it.
            unsafe { FAILED = true };
        }
    });
    // SAFETY: only written inside call_once above.
    if unsafe { FAILED } {
        anyhow::bail!("GStreamer failed to initialise");
    }
    Ok(())
}

pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub size: (u32, u32),
}

#[derive(Debug)]
pub enum VideoEvent {
    /// A new frame is ready (`take_frame`).
    Frame,
    Eos,
    Error(String),
}

/// Copy a mapped RGBA frame into a tight buffer (rows may be padded).
fn frame_from_sample(sample: &gst::Sample) -> Option<VideoFrame> {
    let caps = sample.caps()?;
    let info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let (w, h) = (info.width(), info.height());
    let stride = *info.stride().first()? as usize;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let row = w as usize * 4;
    let mut rgba = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(map.get(y * stride..y * stride + row)?);
    }
    Some(VideoFrame { rgba, size: (w, h) })
}

fn rgba_sink() -> gst_app::AppSink {
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Rgba)
        .build();
    gst_app::AppSink::builder()
        .caps(&caps)
        .max_buffers(2)
        .drop(true)
        .build()
}

pub struct Player {
    pipeline: gst::Element,
    latest: Arc<Mutex<Option<VideoFrame>>>,
    /// A Frame event is in flight; don't flood the event loop.
    pending: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    paused: bool,
    /// Last known position: queries fail briefly after a seek while paused.
    last_pos: Cell<f64>,
}

impl Player {
    /// Start playing `uri` (file:// or http(s)://). `id` tags its events.
    pub fn new(
        uri: &str,
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        volume: f64,
        muted: bool,
    ) -> anyhow::Result<Self> {
        init()?;
        let latest: Arc<Mutex<Option<VideoFrame>>> = Arc::default();
        let pending = Arc::new(AtomicBool::new(false));
        let sink = rgba_sink();
        {
            let (latest, pending, proxy) = (latest.clone(), pending.clone(), proxy.clone());
            sink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |s| {
                        let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        if let Some(frame) = frame_from_sample(&sample) {
                            *latest.lock().unwrap() = Some(frame);
                            if !pending.swap(true, Ordering::AcqRel) {
                                let _ = proxy.send_event(UserEvent::Video {
                                    id,
                                    event: VideoEvent::Frame,
                                });
                            }
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri)
            .property("video-sink", &sink)
            .property("volume", volume.clamp(0.0, 1.5))
            .property("mute", muted)
            .build()?;

        // Registered headers (e.g. a media server's API key) for HTTP streams.
        // ...and the proxy (wlens.opt.proxy / *_PROXY), so streams go the same way.
        let headers = crate::io::remote::registered_headers(uri);
        let http_proxy = crate::io::remote::proxy();
        if !headers.is_empty() || http_proxy.is_some() {
            pipeline.connect("source-setup", false, move |args| {
                let src = args.get(1).and_then(|v| v.get::<gst::Element>().ok())?;
                if !headers.is_empty() && src.find_property("extra-headers").is_some() {
                    let mut s = gst::Structure::builder("extra-headers");
                    for (k, v) in &headers {
                        s = s.field(k.as_str(), v.as_str());
                    }
                    src.set_property("extra-headers", s.build());
                }
                if let Some(p) = &http_proxy
                    && src.find_property("proxy").is_some()
                {
                    src.set_property("proxy", p.as_str());
                }
                None
            });
        }

        // End-of-stream and errors arrive on the bus; watch it from a thread.
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(bus) = pipeline.bus() {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("wlens-video-bus".into())
                .spawn(move || {
                    use gst::MessageView;
                    while !stop.load(Ordering::Acquire) {
                        let Some(msg) = bus.timed_pop_filtered(
                            gst::ClockTime::from_mseconds(200),
                            &[gst::MessageType::Eos, gst::MessageType::Error],
                        ) else {
                            continue;
                        };
                        let event = match msg.view() {
                            MessageView::Eos(_) => VideoEvent::Eos,
                            MessageView::Error(e) => VideoEvent::Error(e.error().to_string()),
                            _ => continue,
                        };
                        if proxy.send_event(UserEvent::Video { id, event }).is_err() {
                            break;
                        }
                    }
                })?;
        }
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            // Don't leave the bus thread or the pipeline running.
            stop.store(true, Ordering::Release);
            let _ = pipeline.set_state(gst::State::Null);
            return Err(e.into());
        }
        Ok(Self {
            pipeline,
            latest,
            pending,
            stop,
            paused: false,
            last_pos: Cell::new(0.0),
        })
    }

    /// The newest decoded frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<VideoFrame> {
        self.pending.store(false, Ordering::Release);
        self.latest.lock().unwrap().take()
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        let state = if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        if let Err(e) = self.pipeline.set_state(state) {
            log::warn!("video: {e}");
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn position(&self) -> Option<f64> {
        match self.pipeline.query_position::<gst::ClockTime>() {
            Some(t) if t.nseconds() > 0 || !self.paused => {
                let p = t.nseconds() as f64 / 1e9;
                self.last_pos.set(p);
                Some(p)
            }
            _ => Some(self.last_pos.get()),
        }
    }

    pub fn duration(&self) -> Option<f64> {
        self.pipeline
            .query_duration::<gst::ClockTime>()
            .map(|t| t.nseconds() as f64 / 1e9)
    }

    pub fn seek(&self, secs: f64) {
        self.last_pos.set(secs.max(0.0));
        let t = gst::ClockTime::from_nseconds((secs.max(0.0) * 1e9) as u64);
        // ACCURATE: land where asked, not on the previous keyframe (which can
        // be the start of a short clip).
        if let Err(e) = self
            .pipeline
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, t)
        {
            log::debug!("video seek: {e}");
        }
    }

    pub fn set_volume(&self, volume: f64, muted: bool) {
        self.pipeline.set_property("volume", volume.clamp(0.0, 1.5));
        self.pipeline.set_property("mute", muted);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// A still from roughly 10% into a local video, for gallery thumbnails.
pub fn thumbnail(path: &Path) -> anyhow::Result<image::DynamicImage> {
    init()?;
    let uri = gst::glib::filename_to_uri(path, None)?;
    let sink = rgba_sink();
    let fakeaudio = gst::ElementFactory::make("fakesink").build()?;
    let pipeline = gst::ElementFactory::make("playbin")
        .property("uri", uri.as_str())
        .property("video-sink", &sink)
        .property("audio-sink", &fakeaudio)
        .build()?;
    let _guard = scopeguard(&pipeline);
    pipeline.set_state(gst::State::Paused)?;
    let (res, _, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    res?;
    if let Some(d) = pipeline.query_duration::<gst::ClockTime>() {
        let at = gst::ClockTime::from_nseconds(d.nseconds() / 10);
        let _ = pipeline.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, at);
        let _ = pipeline.state(gst::ClockTime::from_seconds(5));
    }
    let sample = sink
        .try_pull_preroll(gst::ClockTime::from_seconds(5))
        .ok_or_else(|| anyhow::anyhow!("no video frame"))?;
    let f = frame_from_sample(&sample).ok_or_else(|| anyhow::anyhow!("unreadable frame"))?;
    let img = image::RgbaImage::from_raw(f.size.0, f.size.1, f.rgba)
        .ok_or_else(|| anyhow::anyhow!("bad frame size"))?;
    Ok(image::DynamicImage::ImageRgba8(img))
}

/// Set the pipeline to Null when the thumbnailer returns, however it returns.
fn scopeguard(p: &gst::Element) -> impl Drop + '_ {
    struct Guard<'a>(&'a gst::Element);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let _ = self.0.set_state(gst::State::Null);
        }
    }
    Guard(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_video_name("clip.webm"));
        assert!(is_video_name("https://i.example.org/1234.MP4?x=1"));
        assert!(!is_video_name("photo.png"));
        assert!(!is_video_name("no_extension"));
    }
}
