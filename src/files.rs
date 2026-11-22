//! Turning command-line arguments into the starting item list.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::model::item::{Item, Source};

/// What the command line asked for.
pub struct Request<'a> {
    pub inputs: &'a [PathBuf],
    pub recursive: bool,
    pub siblings: bool,
    /// 1-based.
    pub start: usize,
}

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .and_then(image::ImageFormat::from_extension)
        .is_some_and(|f| f.reading_enabled())
}

/// A video we can play (always false without the `video` feature).
pub fn is_video(p: &Path) -> bool {
    #[cfg(feature = "video")]
    {
        crate::io::video::is_video_name(&p.to_string_lossy())
    }
    #[cfg(not(feature = "video"))]
    {
        let _ = p;
        false
    }
}

fn scan_dir(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return log::warn!("cannot read {}", dir.display());
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                scan_dir(&p, true, out);
            }
        } else if is_image(&p) || is_video(&p) {
            out.push(p);
        }
    }
}

fn paths_to_items(paths: Vec<PathBuf>) -> Vec<Item> {
    paths
        .into_iter()
        .map(|p| Item::new(Source::Path(p)))
        .collect()
}

/// Expand arguments (files, folders, URLs, `-` for stdin) into items, plus
/// the index to start at. With no arguments, the current folder.
pub fn gather(req: &Request) -> anyhow::Result<(Vec<Item>, usize)> {
    let mut inputs = Vec::new();
    for f in req.inputs {
        if f.as_os_str() == "-" {
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                if !line.is_empty() {
                    inputs.push(line);
                }
            }
        } else {
            inputs.push(f.to_string_lossy().into_owned());
        }
    }
    if inputs.is_empty() {
        inputs.push(".".into());
    }

    // One file with --siblings: its whole folder, starting at that file.
    if req.siblings && inputs.len() == 1 && Path::new(&inputs[0]).is_file() {
        let file = std::fs::canonicalize(&inputs[0])?;
        let mut files = Vec::new();
        scan_dir(file.parent().unwrap_or(Path::new("/")), false, &mut files);
        let start = files.iter().position(|p| *p == file).unwrap_or(0);
        return Ok((paths_to_items(files), start));
    }

    let mut items = Vec::new();
    for input in inputs {
        let src = Source::parse(&input);
        match &src {
            Source::Url(_) => items.push(Item::new(src)),
            Source::Path(p) if p.is_dir() => {
                let mut files = Vec::new();
                scan_dir(p, req.recursive, &mut files);
                items.extend(paths_to_items(files));
            }
            // Explicitly named files are tried regardless of extension.
            Source::Path(p) if p.exists() => items.push(Item::new(src)),
            Source::Path(p) => log::warn!("{}: no such file", p.display()),
        }
    }
    Ok((items, req.start.saturating_sub(1)))
}
