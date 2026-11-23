//! Turning command-line arguments into the starting item list.

use std::cmp::Ordering;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::io::archive;
use crate::model::item::{Item, Source};

/// Compare digit runs by magnitude without converting them to integers.
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    fn number(bytes: &[u8]) -> (&[u8], &[u8]) {
        let length = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
        let (digits, rest) = bytes.split_at(length);
        let zeroes = digits.iter().take_while(|&&c| c == b'0').count();
        (&digits[zeroes..], rest)
    }

    let (mut a, mut b) = (left.as_bytes(), right.as_bytes());
    while let (Some(&x), Some(&y)) = (a.first(), b.first()) {
        let order = if x.is_ascii_digit() && y.is_ascii_digit() {
            let (left_number, left_rest) = number(a);
            let (right_number, right_rest) = number(b);
            a = left_rest;
            b = right_rest;
            left_number
                .len()
                .cmp(&right_number.len())
                .then_with(|| left_number.cmp(right_number))
        } else {
            a = &a[1..];
            b = &b[1..];
            x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase())
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len()).then_with(|| left.cmp(right))
}

pub fn location_name(path: &Path) -> String {
    let resolved;
    let path = if path.file_name().is_none() {
        resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        resolved.as_path()
    } else {
        path
    };
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

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
    entries.sort_by(|a, b| {
        natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()).then_with(|| a.cmp(b))
    });
    for p in entries {
        if p.is_dir() {
            if recursive && !p.is_symlink() {
                scan_dir(&p, true, out);
            }
        } else if is_image(&p) || is_video(&p) {
            out.push(p);
        }
    }
}

pub fn flat_directory(path: &Path) -> Vec<Item> {
    let mut files = Vec::new();
    scan_dir(path, true, &mut files);
    paths_to_items(files)
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
    if req.siblings
        && inputs.len() == 1
        && Path::new(&inputs[0]).is_file()
        && !archive::is_archive(Path::new(&inputs[0]))
    {
        let file = std::fs::canonicalize(&inputs[0])?;
        let mut files = Vec::new();
        scan_dir(file.parent().unwrap_or(Path::new("/")), false, &mut files);
        let start = files.iter().position(|p| *p == file).unwrap_or(0);
        return Ok((paths_to_items(files), start));
    }

    let single = inputs.len() == 1;
    let mut items = Vec::new();
    for input in inputs {
        let src = Source::parse(&input);
        match &src {
            Source::Url(_) | Source::Archive { .. } => items.push(Item::new(src)),
            Source::Path(p) if p.is_dir() => {
                let mut files = Vec::new();
                scan_dir(p, req.recursive, &mut files);
                items.extend(paths_to_items(files));
            }
            Source::Path(p) if archive::is_archive(p) && p.is_file() => match archive::list(p) {
                Ok(pages) => items.extend(pages.into_iter().map(Item::new)),
                Err(e) if single => return Err(e),
                Err(e) => log::warn!("{}: {e:#}", p.display()),
            },
            // Other explicitly named files are tried regardless of extension.
            Source::Path(p) if p.exists() => items.push(Item::new(src)),
            Source::Path(p) => log::warn!("{}: no such file", p.display()),
        }
    }
    Ok((items, req.start.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_archive_expands_with_start_and_siblings() {
        let inputs = [PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages.cbz")];
        for siblings in [false, true] {
            let (items, index) = gather(&Request {
                inputs: &inputs,
                recursive: false,
                siblings,
                start: 2,
            })
            .unwrap();
            assert_eq!(items.len(), 3);
            assert_eq!(index, 1);
            assert_eq!(items[index].name(), "page10.png");
        }
    }

    #[test]
    fn natural_order_handles_long_numbers_and_breaks_ties() {
        let mut names = ["page11", "Page9999999999999999999999999", "page2", "page02"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["page02", "page2", "page11", "Page9999999999999999999999999"]
        );
    }
}
