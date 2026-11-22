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
            folders.entry(first).or_insert_with(|| {
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
        assert!(items[2].archive_folder.as_ref().unwrap().items().is_empty());
    }
}
