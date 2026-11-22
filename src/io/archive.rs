//! List image pages from headers and materialize them in one sequential pass.
//! Only generated names are used on disk. An archive entry is never a destination path.

use std::collections::HashSet;
use std::fs::{self, File, Metadata};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use compress_tools::{ArchiveContents, ArchiveIteratorBuilder};

use crate::model::item::Source;

struct Limits {
    entries: usize,
    page: u64,
    total: u64,
}

const LIMITS: Limits = Limits {
    entries: 10_000,
    page: 256 << 20,
    total: 2 << 30,
};

pub fn is_archive(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    [
        ".cbz", ".zip", ".cbr", ".rar", ".cb7", ".7z", ".cbt", ".tar", ".tar.gz", ".tgz",
        ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar.zst",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

pub fn list(path: &Path) -> Result<Vec<Source>> {
    let archive = fs::canonicalize(path)?;
    let mut names = walk(File::open(&archive)?, None, &LIMITS)
        .with_context(|| format!("listing {}", archive.display()))?;
    ensure!(
        !names.is_empty(),
        "{} contains no image pages",
        archive.display()
    );
    names.sort_unstable_by(|a, b| crate::files::natural_cmp(a, b));
    Ok(names
        .into_iter()
        .map(|entry| Source::Archive {
            archive: archive.clone(),
            entry,
        })
        .collect())
}

fn page_name(name: &str) -> bool {
    let path = Path::new(name);
    crate::files::is_image(path)
        && path.components().all(|c| match c {
            Component::Normal(n) => n != "__MACOSX" && !n.as_encoded_bytes().starts_with(b"._"),
            Component::CurDir => true,
            _ => false,
        })
}

fn filename(entry: &str) -> Result<String> {
    ensure!(page_name(entry), "invalid archive page name");
    let ext = Path::new(entry).extension().unwrap().to_str().unwrap();
    Ok(format!(
        "{:x}.{}",
        md5::compute(entry),
        ext.to_ascii_lowercase()
    ))
}

// Include inode and nanosecond timestamps so replacing or editing an archive
// invalidates both extracted pages and thumbnails, even within one second.
pub(super) fn revision(meta: &Metadata) -> String {
    let id = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec()
    );
    format!("{:x}", md5::compute(id))
}

fn root() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .context("no cache directory")?
        .join("wlens/archives"))
}

pub fn cached_path(archive: &Path, entry: &str) -> Result<PathBuf> {
    Ok(root()?
        .join(revision(&fs::metadata(archive)?))
        .join("pages")
        .join(filename(entry)?))
}

/// Called on loader workers. A directory lock also coalesces requests from
/// other viewer processes. Publishing by rename hides incomplete extractions.
pub fn materialize(archive: &Path, entry: &str) -> Result<PathBuf> {
    materialize_in(archive, entry, &root()?)
        .with_context(|| format!("reading {}", archive.display()))
}

fn materialize_in(archive: &Path, entry: &str, root: &Path) -> Result<PathBuf> {
    let file = File::open(archive)?;
    let key = revision(&file.metadata()?);
    let dir = root.join(&key);
    let pages = dir.join("pages");
    let dest = pages.join(filename(entry)?);
    if !pages.is_dir() {
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&dir)?;
        let lock = File::open(&dir)?;
        lock.lock()?;
        if !pages.is_dir() {
            let staging = dir.join("pending");
            // Reclaim any staging data left by an interrupted viewer.
            if staging.exists() {
                fs::remove_dir_all(&staging)?;
            }
            fs::create_dir(&staging)?;
            let result = (|| -> Result<()> {
                walk(file, Some(&staging), &LIMITS)?;
                ensure!(
                    revision(&fs::metadata(archive)?) == key,
                    "archive changed during extraction"
                );
                fs::rename(&staging, &pages)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            result?;
        }
    }
    ensure!(dest.is_file(), "missing archive page {entry}");
    Ok(dest)
}

fn walk(file: File, output: Option<&Path>, limits: &Limits) -> Result<Vec<String>> {
    let mut iter = ArchiveIteratorBuilder::new(file)
        .mtree_format(false)
        .build()?;
    let mut names = HashSet::new();
    let mut headers = 0;
    let mut total = 0;
    while let Some(header) = iter.next_header() {
        let (name, stat) = match header {
            ArchiveContents::StartOfEntry(name, stat) => (name, stat),
            ArchiveContents::Err(e) => return Err(e.into()),
            _ => bail!("unexpected archive header"),
        };
        headers += 1;
        ensure!(headers <= limits.entries, "too many archive entries");
        ensure!(name.len() <= 4096, "archive entry name too long");
        let remaining = limits.total - total;
        let declared = stat.st_size.max(0) as u64;
        ensure!(declared <= remaining, "archive exceeds total size limit");
        total += declared;
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG || !page_name(&name) {
            continue;
        }
        ensure!(
            declared <= limits.page,
            "archive page exceeds size limit: {name}"
        );
        let target = output
            .map(|dir| filename(&name).map(|f| dir.join(f)))
            .transpose()?;
        if !names.insert(name) {
            continue;
        }
        let Some(target) = target else { continue };
        let mut page = File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(target)?;
        let capacity = limits.page.min(remaining);
        let written = copy_page(&mut iter, &mut page, capacity)?;
        ensure!(written >= declared, "truncated archive page");
        total += written.saturating_sub(declared);
    }
    iter.close()?;
    Ok(names.into_iter().collect())
}

// Enforce the remaining budget before each write, even when a header
// omits or understates the uncompressed size.
fn copy_page(
    iter: &mut impl Iterator<Item = ArchiveContents>,
    page: &mut impl Write,
    capacity: u64,
) -> Result<u64> {
    let mut written = 0;
    for event in iter {
        match event {
            ArchiveContents::DataChunk(chunk) => {
                ensure!(
                    chunk.len() as u64 <= capacity - written,
                    "archive data exceeds size limit"
                );
                page.write_all(&chunk)?;
                written += chunk.len() as u64;
            }
            ArchiveContents::EndOfEntry => return Ok(written),
            ArchiveContents::Err(e) => return Err(e.into()),
            ArchiveContents::StartOfEntry(..) => break,
        }
    }
    bail!("incomplete archive page")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(ext: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/pages.{ext}"))
    }

    fn temp_dir(test: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("wlens-archive-{test}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn comics_list_and_extract_the_same_three_pages() {
        let root = temp_dir("comics");
        for ext in ["cbz", "cb7", "cbr", "tar.gz"] {
            let archive = fixture(ext);
            let pages = list(&archive).unwrap();
            let expected = [
                ("chapter2/page2.png", [255, 0, 0]),
                ("chapter2/page10.png", [0, 255, 0]),
                ("chapter10/page2.png", [0, 0, 255]),
            ];
            assert_eq!(pages.len(), expected.len(), "{ext}");
            // Simultaneous image and thumbnail requests for distinct pages.
            std::thread::scope(|scope| {
                for (page, (name, rgb)) in pages.iter().zip(expected) {
                    let Source::Archive { archive, entry } = page else {
                        panic!()
                    };
                    assert_eq!(entry, name);
                    assert_eq!(Source::parse(&page.as_string()), *page);
                    let root = &root;
                    scope.spawn(move || {
                        let dest = materialize_in(archive, entry, root).unwrap();
                        assert_eq!(dest.file_name().unwrap().len(), 36);
                        assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 3);
                        let img = crate::io::loader::decode_image(&dest).unwrap().into_rgb8();
                        assert_eq!(img.get_pixel(0, 0).0, rgb);
                        let mtime = fs::metadata(&dest).unwrap().modified().unwrap();
                        materialize_in(archive, entry, root).unwrap();
                        assert_eq!(fs::metadata(dest).unwrap().modified().unwrap(), mtime);
                    });
                }
            });
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn limits_apply_to_headers_and_payloads() {
        for (limits, text) in [
            (
                Limits {
                    entries: 1,
                    ..LIMITS
                },
                "too many archive entries",
            ),
            (Limits { page: 5, ..LIMITS }, "page exceeds size limit"),
            (
                Limits {
                    total: 100,
                    ..LIMITS
                },
                "archive exceeds total size limit",
            ),
        ] {
            let err = walk(File::open(fixture("cbz")).unwrap(), None, &limits).unwrap_err();
            assert!(err.to_string().contains(text), "{err}");
        }
    }

    #[test]
    fn streamed_data_cannot_exceed_remaining_budget() {
        let mut chunks = [
            ArchiveContents::DataChunk(vec![1, 2]),
            ArchiveContents::DataChunk(vec![3, 4]),
            ArchiveContents::EndOfEntry,
        ]
        .into_iter();
        let mut output = Vec::new();
        assert!(copy_page(&mut chunks, &mut output, 3).is_err());
        assert_eq!(output, [1, 2]);
    }

    #[test]
    fn failed_pass_can_retry_after_archive_replacement() {
        let root = temp_dir("retry");
        let archive = root.join("book.cbz");
        fs::write(&archive, b"invalid").unwrap();
        assert!(materialize_in(&archive, "chapter2/page2.png", &root).is_err());
        let failed = root.join(revision(&fs::metadata(&archive).unwrap()));
        assert_eq!(fs::read_dir(failed).unwrap().count(), 0);
        fs::copy(fixture("cbz"), &archive).unwrap();
        let first = materialize_in(&archive, "chapter2/page2.png", &root).unwrap();
        fs::copy(fixture("cb7"), &archive).unwrap();
        let second = materialize_in(&archive, "chapter2/page2.png", &root).unwrap();
        assert_ne!(first, second);
        fs::remove_dir_all(root).unwrap();
    }
}
