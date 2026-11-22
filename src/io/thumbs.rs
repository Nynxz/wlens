//! Thumbnails per the freedesktop.org Thumbnail Managing Standard, so they're
//! shared with file managers: `$XDG_CACHE_HOME/thumbnails/<size>/<md5(uri)>.png`
//! with `Thumb::URI` and `Thumb::MTime` text chunks for invalidation.
//! <https://specifications.freedesktop.org/thumbnail-spec/latest/>

use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use crate::model::item::Source;

/// Spec size buckets: (directory, max edge in px).
pub const LEVELS: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

/// The smallest bucket that's at least `px` big.
pub fn level_for(px: u32) -> u32 {
    LEVELS
        .iter()
        .map(|l| l.1)
        .find(|&s| s >= px)
        .unwrap_or(1024)
}

fn level_dir(size: u32) -> &'static str {
    LEVELS.iter().find(|l| l.1 == size).map_or("large", |l| l.0)
}

pub struct Thumb {
    pub rgba: Vec<u8>,
    pub size: (u32, u32),
    /// Dimensions of the full image, if known (`Thumb::Image::Width/Height`).
    pub orig_size: Option<(u32, u32)>,
}

pub fn root() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("thumbnails"))
}

pub fn thumb_file(uri: &str, size: u32) -> Option<PathBuf> {
    root().map(|r| {
        r.join(level_dir(size))
            .join(format!("{:x}.png", md5::compute(uri.as_bytes())))
    })
}

fn mtime(p: &Path) -> Option<u64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
}

/// Load a cached thumbnail if it's still valid, else generate (and cache) one.
/// `from` is where to get pixels when it differs from the item itself (a
/// media server's own thumbnail URL); the cache is still keyed by `src`.
pub fn load(src: &Source, from: Option<&Source>, size: u32) -> anyhow::Result<Thumb> {
    // Key the cache on the file we actually thumbnail: for a container's
    // cover or a server thumbnail that's `from`, which keeps every entry a
    // genuine image thumbnail in the shared freedesktop cache.
    let key = from.unwrap_or(src);
    let uri = key.uri();
    let file = thumb_file(&uri, size);
    let src_mtime = key.local_path().and_then(mtime);

    if let Some(f) = &file
        && let Some(t) = read_cached(f, &uri, src_mtime)
    {
        return Ok(t);
    }

    let local = match from.unwrap_or(src) {
        Source::Path(p) => p.clone(),
        Source::Url(u) => crate::io::remote::fetch_to_cache(u)?,
    };
    #[cfg(feature = "video")]
    let img = if crate::files::is_video(&local) {
        crate::io::video::thumbnail(&local)?
    } else {
        crate::io::loader::decode_image(&local)?
    };
    #[cfg(not(feature = "video"))]
    let img = crate::io::loader::decode_image(&local)?;
    // A server thumbnail's size isn't the image's; only trust our own decode.
    let orig = (img.width(), img.height());
    let orig_known = from.is_none();
    // Never upscale: small images are stored as-is (per spec).
    let thumb = if orig.0 > size || orig.1 > size {
        img.thumbnail(size, size)
    } else {
        img
    };
    let rgba = thumb.to_rgba8();

    // Don't thumbnail our own thumbnails.
    let inside_cache = root()
        .zip(key.local_path())
        .is_some_and(|(r, p)| p.starts_with(r));
    if let Some(f) = &file
        && !inside_cache
        && let Err(e) = write_cached(f, &uri, src_mtime, &rgba, orig_known.then_some(orig), key)
    {
        log::debug!("could not cache thumbnail for {uri}: {e}");
    }
    Ok(Thumb {
        size: rgba.dimensions(),
        rgba: rgba.into_raw(),
        orig_size: orig_known.then_some(orig),
    })
}

fn read_cached(f: &Path, uri: &str, src_mtime: Option<u64>) -> Option<Thumb> {
    let dec = png::Decoder::new(BufReader::new(std::fs::File::open(f).ok()?));
    let reader = dec.read_info().ok()?;
    let text = &reader.info().uncompressed_latin1_text;
    let get = |k: &str| {
        text.iter()
            .find(|c| c.keyword == k)
            .map(|c| c.text.as_str())
    };
    if get("Thumb::URI") != Some(uri) {
        return None;
    }
    // Local files must match their mtime; remote ones are immutable by URL.
    if let Some(m) = src_mtime
        && get("Thumb::MTime").and_then(|t| t.parse::<u64>().ok()) != Some(m)
    {
        return None;
    }
    let dim = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
    let orig_size = dim("Thumb::Image::Width").zip(dim("Thumb::Image::Height"));
    drop(reader);
    let img = image::open(f).ok()?.to_rgba8();
    Some(Thumb {
        size: img.dimensions(),
        rgba: img.into_raw(),
        orig_size,
    })
}

fn write_cached(
    f: &Path,
    uri: &str,
    src_mtime: Option<u64>,
    rgba: &image::RgbaImage,
    orig: Option<(u32, u32)>,
    src: &Source,
) -> anyhow::Result<()> {
    let dir = f.parent().unwrap();
    create_private_dir(dir)?;
    let tmp = dir.join(format!(
        ".wlens-{}-{}.png",
        std::process::id(),
        rand_suffix()
    ));
    {
        let w = BufWriter::new(create_private_file(&tmp)?);
        let mut enc = png::Encoder::new(w, rgba.width(), rgba.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.add_text_chunk("Thumb::URI".into(), uri.into())?;
        if let Some(m) = src_mtime {
            enc.add_text_chunk("Thumb::MTime".into(), m.to_string())?;
        }
        if let Some(len) = src
            .local_path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
        {
            enc.add_text_chunk("Thumb::Size".into(), len.to_string())?;
        }
        if let Some((w, h)) = orig {
            enc.add_text_chunk("Thumb::Image::Width".into(), w.to_string())?;
            enc.add_text_chunk("Thumb::Image::Height".into(), h.to_string())?;
        }
        enc.add_text_chunk("Software".into(), "wlens".into())?;
        let mut writer = enc.write_header()?;
        writer.write_image_data(rgba.as_raw())?;
        writer.finish()?;
    }
    // Atomic replace, as the spec asks, so readers never see half a file.
    std::fs::rename(&tmp, f).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::time::SystemTime::now())
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

fn create_private_file(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example_hash() {
        // The worked example from the spec.
        let f = thumb_file("file:///home/jens/photos/me.png", 128).unwrap();
        assert!(
            f.ends_with("normal/c6ee772d9e49320e97ec29a7eb5b1697.png"),
            "{f:?}"
        );
    }

    #[test]
    fn levels() {
        assert_eq!(level_for(100), 128);
        assert_eq!(level_for(240), 256);
        assert_eq!(level_for(5000), 1024);
    }

    #[test]
    fn roundtrip_through_cache() {
        let dir = std::env::temp_dir().join(format!("wlens-thumb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src_file = dir.join("big.png");
        image::RgbImage::from_pixel(600, 300, image::Rgb([200, 30, 30]))
            .save(&src_file)
            .unwrap();
        let src = Source::Path(src_file.clone());
        let uri = src.uri();
        let out = dir.join("t.png");
        let rgba = image::RgbaImage::from_pixel(256, 128, image::Rgba([1, 2, 3, 255]));
        write_cached(&out, &uri, mtime(&src_file), &rgba, Some((600, 300)), &src).unwrap();
        let t = read_cached(&out, &uri, mtime(&src_file)).expect("valid cache entry");
        assert_eq!((t.size, t.orig_size), ((256, 128), Some((600, 300))));
        assert!(
            read_cached(&out, "file:///other", mtime(&src_file)).is_none(),
            "uri must match"
        );
        assert!(
            read_cached(&out, &uri, Some(1)).is_none(),
            "stale mtime is rejected"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
