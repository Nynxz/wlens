//! Virtual folders retain one shared archive listing across navigation levels.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::item::{Item, Source};

#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveFolder {
    entries: Arc<[Source]>,
    prefix: PathBuf,
}

impl ArchiveFolder {
    pub fn root(entries: Vec<Source>) -> Self {
        Self {
            entries: entries.into(),
            prefix: PathBuf::new(),
        }
    }

    pub fn opened(path: &Path, entries: Vec<Source>) -> Self {
        let root = Self::root(entries);
        let items = root.items();
        if let [only] = items.as_slice()
            && let Some(folder) = &only.archive_folder
            && crate::io::archive::display_stem(path)
                .is_some_and(|name| name.eq_ignore_ascii_case(&only.name()))
        {
            return folder.clone();
        }
        root
    }

    pub fn flat_items(&self) -> Vec<Item> {
        self.entries
            .iter()
            .filter(|src| match src {
                Source::Archive { entry, .. } => !entry.ends_with('/') && self.contains(entry),
                _ => false,
            })
            .cloned()
            .map(Item::new)
            .collect()
    }

    pub fn contains(&self, entry: &str) -> bool {
        let path = Path::new(entry);
        path.strip_prefix(".")
            .unwrap_or(path)
            .starts_with(&self.prefix)
    }

    pub fn items(&self) -> Vec<Item> {
        let mut folders = HashMap::new();
        let mut files = Vec::new();
        for src in self.entries.iter() {
            let Source::Archive { archive, entry } = src else {
                continue;
            };
            let path = Path::new(entry);
            let path = path.strip_prefix(".").unwrap_or(path);
            let Ok(relative) = path.strip_prefix(&self.prefix) else {
                continue;
            };
            let mut parts = relative.iter();
            let Some(first) = parts.next() else { continue };
            if parts.next().is_none() && !entry.ends_with('/') {
                files.push(Item::new(src.clone()));
                continue;
            }
            let folder = folders.entry(first).or_insert_with(|| {
                let prefix = self.prefix.join(first);
                let mut item = Item::new(Source::Archive {
                    archive: archive.clone(),
                    entry: format!("{}/", prefix.display()),
                });
                item.label = Some(first.to_string_lossy().into_owned());
                item.container = true;
                item.icon = Some("\u{f024b}".into());
                item.archive_folder = Some(Self {
                    entries: self.entries.clone(),
                    prefix,
                });
                item
            });
            if folder.thumb.is_none() && !entry.ends_with('/') {
                folder.thumb = Some(src.clone());
            }
        }
        let mut folders: Vec<_> = folders.into_values().collect();
        folders.sort_unstable_by(|a, b| crate::files::natural_cmp(&a.name(), &b.name()));
        files.sort_unstable_by(|a, b| crate::files::natural_cmp(&a.name(), &b.name()));
        folders.extend(files);
        folders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_only_a_sole_wrapper_matching_the_archive_name() {
        let path = Path::new("/My Photos.tar.gz");
        let entries = |names: &[&str]| {
            names
                .iter()
                .map(|s| Source::Archive {
                    archive: path.into(),
                    entry: s.to_string(),
                })
                .collect()
        };
        let root = ArchiveFolder::opened(path, entries(&["My Photos/", "My Photos/trip/p.png"]));
        assert_eq!(root.items()[0].name(), "trip");
        let root = ArchiveFolder::opened(path, entries(&["trip/p.png"]));
        assert_eq!(root.items()[0].name(), "trip");
        let root = ArchiveFolder::opened(path, entries(&["My Photos/p.png", "outside.png"]));
        assert_eq!(root.items().len(), 2);
    }

    #[test]
    fn folders_first_at_each_level() {
        let entries = [
            "z.png",
            "chapter10/p.jpg",
            "chapter2/inner/p.png",
            "./chapter2/p10.jpg",
            "chapter2/p2.jpg",
            "empty/",
        ];
        let root = ArchiveFolder::root(
            entries
                .iter()
                .map(|e| Source::Archive {
                    archive: "/a.zip".into(),
                    entry: e.to_string(),
                })
                .collect(),
        );
        let items = root.items();
        assert_eq!(
            items.iter().map(Item::name).collect::<Vec<_>>(),
            ["chapter2", "chapter10", "empty", "z.png"]
        );
        let chapter = items[0].archive_folder.as_ref().unwrap().items();
        assert_eq!(
            chapter.iter().map(Item::name).collect::<Vec<_>>(),
            ["inner", "p2.jpg", "p10.jpg"]
        );
        assert_eq!(
            chapter[0].archive_folder.as_ref().unwrap().items()[0].name(),
            "p.png"
        );
        assert_eq!(items[0].thumb.as_ref().unwrap().file_name(), "p.png");
        assert!(items[2].thumb.is_none());
        assert_eq!(
            items[0].archive_folder.as_ref().unwrap().flat_items().len(),
            3
        );
        assert!(items[2].archive_folder.as_ref().unwrap().items().is_empty());
    }
}
