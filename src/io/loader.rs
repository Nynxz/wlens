//! Background work: decoding images, making thumbnails, and HTTP. Lua and
//! rendering stay on the main thread; workers post results back to the
//! event loop.
//!
//! The job queue is a shared deque the main thread re-prioritises on every
//! navigation/scroll, so stale jobs that haven't started yet are dropped.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use image::{DynamicImage, ImageDecoder, ImageReader};
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;
use crate::io::thumbs::Thumb;
use crate::model::item::Source;

/// One frame of an animated image.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

pub struct Decoded {
    /// The (first) frame.
    pub rgba: Vec<u8>,
    /// Size of the pixel buffer (may be downscaled to fit GPU limits).
    pub size: (u32, u32),
    /// Original dimensions after EXIF orientation, for display/zoom.
    pub orig_size: (u32, u32),
    /// Size of the file on disk (the cached download for URLs).
    pub file_size: u64,
    /// All frames (including the first) for animated GIF/WebP/APNG; empty for stills.
    pub frames: Vec<Frame>,
}

impl Decoded {
    pub fn bytes(&self) -> usize {
        self.rgba.len() + self.frames.iter().map(|f| f.rgba.len()).sum::<usize>()
    }
}

pub type LoadResult = Result<Arc<Decoded>, String>;
pub type ThumbResult = Result<Arc<Thumb>, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Full-resolution image for the viewer.
    Image(Source),
    /// Thumbnail at a freedesktop size bucket for an item, optionally made
    /// from a separate (server-side thumbnail) source instead of the item.
    Thumb(Source, Option<Source>, u32),
    /// `wlens.fetch(...)`: an HTTP request answered by id.
    Fetch(u64, crate::io::remote::Request),
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

pub struct Loader {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Loader {
    pub fn new(proxy: EventLoopProxy<UserEvent>, max_dim: u32) -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let n = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .clamp(2, 6);
        let workers = (0..n)
            .map(|i| {
                let shared = shared.clone();
                let proxy = proxy.clone();
                std::thread::Builder::new()
                    .name(format!("wlens-worker-{i}"))
                    .spawn(move || worker(&shared, &proxy, max_dim))
                    .expect("spawn worker")
            })
            .collect();
        Self { shared, workers }
    }

    /// Remove and return queued (not yet started) jobs matching `pred`.
    pub fn take_queued(&self, pred: impl Fn(&Job) -> bool) -> Vec<Job> {
        let mut q = self.shared.0.lock().unwrap();
        let (taken, kept): (VecDeque<Job>, VecDeque<Job>) = q.jobs.drain(..).partition(|j| pred(j));
        q.jobs = kept;
        taken.into()
    }

    /// Queue jobs at the front (highest priority first).
    pub fn request_front(&self, jobs: Vec<Job>) {
        let (lock, cv) = &*self.shared;
        let mut q = lock.lock().unwrap();
        for j in jobs.into_iter().rev() {
            q.jobs.push_front(j);
        }
        cv.notify_all();
    }

    /// Queue jobs at the back.
    pub fn request(&self, jobs: impl IntoIterator<Item = Job>) {
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap().jobs.extend(jobs);
        cv.notify_all();
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap().shutdown = true;
        cv.notify_all();
        // Don't join: a worker may be mid-decode of a huge file and we want
        // the window to close instantly. Threads die with the process.
        self.workers.clear();
    }
}

fn worker(shared: &(Mutex<Queue>, Condvar), proxy: &EventLoopProxy<UserEvent>, max_dim: u32) {
    let (lock, cv) = shared;
    loop {
        let job = {
            let mut q = lock.lock().unwrap();
            loop {
                if q.shutdown {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    break j;
                }
                q = cv.wait(q).unwrap();
            }
        };
        let t = std::time::Instant::now();
        let event = match job {
            Job::Image(src) => {
                let result = guarded(|| decode_source(&src, max_dim))
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                log::debug!("decoded {} in {:?}", src.as_string(), t.elapsed());
                UserEvent::Loaded { src, result }
            }
            Job::Thumb(src, from, level) => {
                let result = guarded(|| crate::io::thumbs::load(&src, from.as_ref(), level))
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                log::trace!("thumb {} in {:?}", src.as_string(), t.elapsed());
                UserEvent::Thumb { src, level, result }
            }
            Job::Fetch(id, req) => UserEvent::Fetched {
                id,
                result: guarded(|| crate::io::remote::fetch(&req))
                    .map(|r| (r.status, r.body, r.headers))
                    .map_err(|e| format!("{e:#}")),
            },
        };
        if proxy.send_event(event).is_err() {
            return; // event loop gone
        }
    }
}

/// Run a job, turning a panic (a decoder bug on a bad file) into an error,
/// so the worker survives and the app hears back instead of waiting forever.
fn guarded<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        Err(anyhow::anyhow!("decoder crashed: {msg}"))
    })
}

/// Open, decode and EXIF-orient an image file.
pub fn decode_image(path: &Path) -> anyhow::Result<DynamicImage> {
    let mut decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn decode_source(src: &Source, max_dim: u32) -> anyhow::Result<Decoded> {
    let path = match src {
        Source::Path(p) => p.clone(),
        Source::Url(u) => crate::io::remote::fetch_to_cache(u)?,
    };
    let result = decode(&path, max_dim);
    if result.is_err() && src.is_remote() {
        // A successful HTTP response can still contain broken media.
        let _ = std::fs::remove_file(&path);
    }
    result
}

/// Animations stop decoding past this many bytes of frames (or frames).
const ANIM_BUDGET: usize = 768 * 1024 * 1024;
const ANIM_MAX_FRAMES: usize = 2000;

/// Frames, buffer size, original size.
type Animation = (Vec<Frame>, (u32, u32), (u32, u32));

/// Decode every frame of an animated GIF, WebP or APNG. `None` for stills.
fn decode_animation(path: &Path, max_dim: u32) -> anyhow::Result<Option<Animation>> {
    use image::AnimationDecoder;
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    use std::io::BufReader;
    let open = || -> std::io::Result<BufReader<std::fs::File>> {
        Ok(BufReader::new(std::fs::File::open(path)?))
    };
    let format = ImageReader::new(open()?).with_guessed_format()?.format();
    let frames = match format {
        Some(image::ImageFormat::Gif) => GifDecoder::new(open()?)?.into_frames(),
        Some(image::ImageFormat::WebP) => {
            let d = WebPDecoder::new(open()?)?;
            if !d.has_animation() {
                return Ok(None);
            }
            d.into_frames()
        }
        Some(image::ImageFormat::Png) => {
            let d = PngDecoder::new(open()?)?;
            if !d.is_apng()? {
                return Ok(None);
            }
            d.apng()?.into_frames()
        }
        _ => return Ok(None),
    };
    let mut out = Vec::new();
    let (mut size, mut orig) = ((0, 0), (0, 0));
    let mut bytes = 0usize;
    for frame in frames {
        let frame = frame?;
        let (n, d) = frame.delay().numer_denom_ms();
        let ms = n.checked_div(d).unwrap_or(0);
        // Browsers treat 0-10ms GIF delays as ~100ms; so do we. Other
        // formats (APNG, WebP) mean what they say.
        let gif = format == Some(image::ImageFormat::Gif);
        let delay_ms = if gif && ms <= 10 { 100 } else { ms.max(1) };
        let mut buf = frame.into_buffer();
        orig = buf.dimensions();
        if buf.width() > max_dim || buf.height() > max_dim {
            let k = max_dim as f64 / buf.width().max(buf.height()) as f64;
            let (w, h) = (
                (buf.width() as f64 * k) as u32,
                (buf.height() as f64 * k) as u32,
            );
            buf = image::imageops::resize(
                &buf,
                w.max(1),
                h.max(1),
                image::imageops::FilterType::Triangle,
            );
        }
        size = buf.dimensions();
        bytes += buf.len();
        out.push(Frame {
            rgba: buf.into_raw(),
            delay_ms,
        });
        if bytes > ANIM_BUDGET || out.len() >= ANIM_MAX_FRAMES {
            log::info!(
                "{}: animation truncated at {} frames",
                path.display(),
                out.len()
            );
            break;
        }
    }
    Ok((out.len() > 1).then_some((out, size, orig)))
}

pub fn decode(path: &Path, max_dim: u32) -> anyhow::Result<Decoded> {
    match decode_animation(path, max_dim) {
        Ok(Some((frames, size, orig_size))) => {
            return Ok(Decoded {
                rgba: frames[0].rgba.clone(),
                size,
                orig_size,
                file_size: std::fs::metadata(path).map_or(0, |m| m.len()),
                frames,
            });
        }
        Ok(None) => {}
        // A broken animation may still decode as a still image below.
        Err(e) => log::debug!("{}: animation decode failed: {e}", path.display()),
    }
    let mut img = decode_image(path)?;
    let orig_size = (img.width(), img.height());
    if img.width() > max_dim || img.height() > max_dim {
        log::info!(
            "{}: {}x{} exceeds GPU texture limit {max_dim}, downscaling",
            path.display(),
            img.width(),
            img.height()
        );
        img = img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle);
    }
    let rgba = img.into_rgba8();
    Ok(Decoded {
        size: rgba.dimensions(),
        rgba: rgba.into_raw(),
        orig_size,
        file_size: std::fs::metadata(path).map_or(0, |m| m.len()),
        frames: Vec::new(),
    })
}
