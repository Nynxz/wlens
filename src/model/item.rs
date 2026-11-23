//! What the viewer browses: a list of files, archive pages or URLs,
//! with an optional label. Scripts can replace the list (`wlens.set_items`),
//! so a gallery can come from anywhere.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Path(PathBuf),
    Url(String),
    Archive { archive: PathBuf, entry: String },
}

impl Source {
    pub fn parse(s: &str) -> Self {
        if s.starts_with("http://") || s.starts_with("https://") {
            Self::Url(s.to_string())
        } else if let Some(uri) = s.strip_prefix("wlens-archive:")
            && let Ok(url) = url::Url::parse(uri)
            && let Ok(archive) = url.to_file_path()
            && let Some((_, entry)) = url.query_pairs().find(|(key, _)| key == "entry")
        {
            Self::Archive {
                archive,
                entry: entry.into_owned(),
            }
        } else {
            Self::Path(PathBuf::from(s.strip_prefix("file://").unwrap_or(s)))
        }
    }

    /// The path or URL as text.
    pub fn as_string(&self) -> String {
        match self {
            Self::Path(p) => p.to_string_lossy().into_owned(),
            Self::Url(u) => u.clone(),
            Self::Archive { archive, entry } => {
                // Archive discovery canonicalizes the path. File URLs also
                // preserve non-UTF-8 bytes in the archive's own name.
                let mut url = url::Url::from_file_path(archive).expect("absolute archive path");
                url.query_pairs_mut().append_pair("entry", entry);
                format!("wlens-archive:{url}")
            }
        }
    }

    /// Last path segment, without a URL's query string.
    pub fn file_name(&self) -> String {
        match self {
            Self::Archive { entry, .. } => entry.rsplit('/').next().unwrap_or(entry).to_owned(),
            Self::Path(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Self::Url(u) => {
                let no_query = u.split(['?', '#']).next().unwrap_or(u);
                no_query
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(no_query)
                    .to_string()
            }
        }
    }

    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Self::Path(p) => Some(p),
            Self::Url(_) | Self::Archive { .. } => None,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Url(_))
    }

    /// The URI the freedesktop thumbnail spec keys on: `file:///abs/path`
    /// (percent-encoded) for files, the URL itself otherwise.
    pub fn uri(&self) -> String {
        match self {
            Self::Archive { .. } => self.as_string(),
            Self::Url(u) => u.clone(),
            Self::Path(p) => {
                let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                format!("file://{}", percent_encode_path(&abs.to_string_lossy()))
            }
        }
    }
}

/// RFC 3986 escaping for a path: keep unreserved characters and `/`.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub src: Source,
    /// Shown instead of the file name (`{name}`), e.g. a title from an API.
    pub label: Option<String>,
    /// Where to get the gallery thumbnail, if not from the image itself
    /// (e.g. a media server's thumbnail endpoint — much cheaper over a network).
    /// For containers this is the cover.
    pub thumb: Option<Source>,
    /// A container (folder, album, gallery…): opening it runs `open`, which
    /// usually pushes a new item list. Containers are skipped in image view.
    pub container: bool,
    pub open: Option<crate::lua::Action>,
    /// Badge on a tile, e.g. how many images a container holds ("120+").
    pub count: Option<String>,
    /// Drawn on a tile with no cover (e.g. a folder glyph).
    pub icon: Option<String>,
    /// Anything a script wants to attach (title, people, tags, rating…):
    /// shown by `{meta.key}` templates and widgets like the details panel.
    pub meta: Option<mlua::Table>,
    /// Play as a video even without a video file extension (e.g. a media
    /// server's `/stream` URL).
    pub video: bool,
}

impl Item {
    pub fn new(src: Source) -> Self {
        Self {
            src,
            label: None,
            thumb: None,
            container: false,
            open: None,
            count: None,
            icon: None,
            meta: None,
            video: false,
        }
    }

    /// A video we can play: flagged by a script, or by its extension.
    pub fn is_video(&self) -> bool {
        (self.video || crate::files::is_video(std::path::Path::new(&self.src.file_name())))
            && cfg!(feature = "video")
    }

    pub fn name(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.src.file_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        assert!(Source::parse("https://x.org/a.png").is_remote());
        assert_eq!(
            Source::parse("file:///tmp/a.png"),
            Source::Path("/tmp/a.png".into())
        );
        assert_eq!(
            Source::parse("https://x.org/img/cat.jpg?w=400#top").file_name(),
            "cat.jpg"
        );
        assert_eq!(
            Source::Path("/tmp/my pic.png".into()).uri(),
            "file:///tmp/my%20pic.png"
        );
    }
}
