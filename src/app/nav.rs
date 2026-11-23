//! Moving through items: navigation, list changes from Lua, levels.

use super::*;

impl App {
    /// Replace or extend the item list (from `wlens.set_items` / `add_items`).
    pub(super) fn apply_items(&mut self, items: Vec<Item>, append: bool, start: Option<usize>) {
        let before = self.items.len();
        if append {
            self.items.extend(items);
        } else {
            self.forward_levels.clear();
            self.forward_image = false;
            self.browser = None;
            self.items = items;
            self.index = start.unwrap_or(0);
            if self.items.is_empty()
                && let (Some(l), Some(r)) = (self.current.take(), &mut self.renderer)
            {
                l.release(r);
            }
        }
        self.index = self.index.min(self.items.len().saturating_sub(1));
        self.script.shared.borrow_mut().items = self.items.clone();
        self.gallery.failed.clear();
        self.gallery.requested = None;
        // Appending (infinite scroll) leaves the scroll where it is: the
        // selection may be far above what you're looking at.
        if self.gallery.active && !append {
            self.ensure_selection_visible();
        }
        self.sync();
        if append && std::mem::take(&mut self.pending_advance) && self.items.len() > before {
            self.navigate(self.index as isize + 1);
        }
        self.redraw();
    }

    /// In image view containers are skipped: walk from `target` in `dir`
    /// to the nearest image. `None` if there isn't one that way.
    pub(super) fn skip_containers(&self, target: isize, dir: isize) -> Option<isize> {
        if self.gallery.active || !self.items.iter().any(|i| i.container) {
            return Some(target);
        }
        let n = self.items.len() as isize;
        let wrap = self.script.shared.borrow().settings.wrap;
        let mut t = target;
        for _ in 0..n {
            let i = if wrap { t.rem_euclid(n) } else { t };
            if !(0..n).contains(&i) {
                return None;
            }
            if !self.items[i as usize].container {
                return Some(t);
            }
            t += dir;
        }
        None
    }

    pub(super) fn navigate_dir(&mut self, target: isize, dir: isize) {
        match self.skip_containers(target, dir) {
            Some(t) => self.navigate(t),
            None if !self.gallery.active => {
                self.toast(if dir < 0 { "First image" } else { "Last image" })
            }
            None => {}
        }
    }

    pub(super) fn navigate(&mut self, target: isize) {
        let n = self.items.len() as isize;
        if n == 0 {
            return;
        }
        let wrap = self.script.shared.borrow().settings.wrap;
        let i = if wrap {
            target.rem_euclid(n)
        } else {
            target.clamp(0, n - 1)
        };
        if i as usize != self.index {
            self.forward_image = false;
            self.nav_dir = if target >= self.index as isize {
                1.0
            } else {
                -1.0
            };
            self.index = i as usize;
            self.pending_advance = false;
            if self.gallery.active {
                self.ensure_selection_visible();
            }
            self.sync();
            self.script.emit("select");
            if !self.gallery.active {
                self.maybe_end_reached(self.index + 3 >= self.items.len());
            }
        } else if target > self.index as isize && !wrap && !self.gallery.active && self.more() {
            // More may be on the way: ask for it and step forward when it lands.
            self.end_fired = None;
            self.maybe_end_reached(true);
            self.pending_advance = true;
            self.toast("Loading more…");
        } else if target != self.index as isize && !wrap && !self.gallery.active {
            self.toast(if target < 0 {
                "First image"
            } else {
                "Last image"
            });
        }
    }

    /// Fire `end_reached` once per (level, list length) when near the end.
    pub(super) fn maybe_end_reached(&mut self, near: bool) {
        let key = (self.stack.len(), self.items.len());
        if near
            && !self.items.is_empty()
            && self.end_fired != Some(key)
            && self.script.has_hooks("end_reached")
        {
            self.end_fired = Some(key);
            self.script.emit("end_reached");
            // Hand any fetches the hook queued to the loader right away.
            self.flush_script();
        }
    }

    pub(super) fn apply_op(&mut self, op: ItemsOp) {
        match op {
            ItemsOp::Set(list, start) => {
                if self.stack.is_empty() {
                    self.script.shared.borrow_mut().level = None;
                }
                self.apply_items(list, false, start);
            }
            ItemsOp::Add(list) => self.apply_items(list, true, None),
            ItemsOp::Push(list, start, title) => self.push_level(list, start, title),
            ItemsOp::Pop => {
                self.back();
            }
            ItemsOp::Directory(path) => {
                if self.stack.is_empty() {
                    self.script.shared.borrow_mut().level =
                        Some(crate::files::location_name(&path));
                }
                self.browser = Some(Browser {
                    location: BrowserLocation::Directory(path),
                    flat: false,
                });
                self.update_browser_view();
                self.update_info();
            }
            ItemsOp::More(more) => {
                self.script.shared.borrow_mut().more = more;
                self.gallery.requested = None;
                self.redraw();
            }
        }
    }

    /// Enter a new list (a folder, an album…), remembering where we were.
    pub(super) fn push_level(
        &mut self,
        items: Vec<Item>,
        start: Option<usize>,
        title: Option<String>,
    ) {
        let parent = self.take_level();
        self.stack.push(parent);
        self.script.shared.borrow_mut().level = title;
        self.gallery.scroll = 0.0;
        self.gallery.active = true;
        self.apply_items(items, false, start);
        self.gallery.shown = self.gallery.scroll;
        self.script.emit("select");
        self.script.emit("level");
    }

    fn take_level(&mut self) -> Level {
        let mut sh = self.script.shared.borrow_mut();
        Level {
            items: std::mem::take(&mut self.items),
            index: self.index,
            scroll: self.gallery.scroll,
            title: sh.level.take(),
            more: std::mem::take(&mut sh.more),
            browser: self.browser.take(),
            forward_image: std::mem::take(&mut self.forward_image),
        }
    }

    /// Go back up a level. Returns false at the top.
    pub(super) fn back(&mut self) -> bool {
        let Some(level) = self.stack.pop() else {
            return false;
        };
        let child = self.take_level();
        self.forward_levels.push(child);
        self.restore_level(level);
        true
    }

    pub(super) fn forward(&mut self) {
        if !self.gallery.active {
            return;
        }
        if self.forward_image {
            self.set_gallery(false);
        } else if let Some(level) = self.forward_levels.pop() {
            let parent = self.take_level();
            self.stack.push(parent);
            self.restore_level(level);
        }
    }

    fn restore_level(&mut self, level: Level) {
        self.listing_archive = None;
        self.browser = level.browser;
        self.forward_image = level.forward_image;
        self.items = level.items;
        self.index = level.index.min(self.items.len().saturating_sub(1));
        self.script.shared.borrow_mut().level = level.title;
        {
            let mut sh = self.script.shared.borrow_mut();
            sh.items = self.items.clone();
            sh.more = level.more;
        }
        self.pending_advance = false;
        self.gallery.active = true;
        self.gallery.scroll = level.scroll;
        self.gallery.shown = level.scroll;
        self.update_browser_view();
        self.gallery.requested = None;
        self.sync();
        self.script.emit("select");
        self.script.emit("level");
        self.redraw();
    }

    pub(super) fn open_archive_folder(
        &mut self,
        folder: crate::model::archive_folder::ArchiveFolder,
        title: Option<String>,
    ) {
        let flat = self.script.shared.borrow().settings.flat;
        let items = if flat {
            folder.flat_items()
        } else {
            folder.items()
        };
        self.push_level(items, None, title);
        self.browser = Some(Browser {
            location: BrowserLocation::Archive(folder),
            flat,
        });
    }

    /// History stores the list as last displayed. Rebuild it only when the
    /// current preference differs, keeping both navigation stacks intact.
    pub(super) fn update_browser_view(&mut self) {
        let flat = self.script.shared.borrow().settings.flat;
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        if browser.flat == flat {
            return;
        }
        let result = match &browser.location {
            BrowserLocation::Archive(folder) => Ok(if flat {
                folder.flat_items()
            } else {
                folder.items()
            }),
            BrowserLocation::Directory(dir) if flat => Ok(crate::files::flat_directory(dir)),
            BrowserLocation::Directory(dir) => self.script.directory_items(dir),
        };
        let items = match result {
            Ok(items) => items,
            Err(err) => {
                self.toast(&format!("Could not open folder: {err}"));
                return;
            }
        };
        let selected = self.items.get(self.index);
        let index = selected.and_then(|old| matching_browser_item(old, &items));
        self.forward_image &= selected
            .zip(index.and_then(|i| items.get(i)))
            .is_some_and(|(old, new)| old.src == new.src && !new.container);
        browser.flat = flat;
        self.items = items;
        self.index = index.unwrap_or(0);
        self.script.shared.borrow_mut().items = self.items.clone();
        self.gallery.active = true;
        self.gallery.requested = None;
        self.gallery.scroll = 0.0;
        self.ensure_selection_visible();
        self.gallery.shown = self.gallery.scroll;
        self.sync();
        self.redraw();
    }

    /// Enter the current container, or view the current image.
    pub(super) fn open_current(&mut self, event_loop: &ActiveEventLoop) {
        let Some(item) = self.items.get(self.index).cloned() else {
            return;
        };
        if !item.container {
            return self.set_gallery(false);
        }
        if let Some(folder) = &item.archive_folder {
            self.open_archive_folder(folder.clone(), Some(item.name()));
            return;
        }
        match item.open {
            Some(Action::Command(c)) => self.exec(event_loop, c),
            Some(Action::Lua(f)) => {
                let info = self.script.info_value();
                self.script.call_with(&f, info, "item open");
            }
            None if let Source::Path(path) = item.src
                && crate::io::archive::is_archive(&path) =>
            {
                if self.listing_archive.as_ref() != Some(&path)
                    && let Some(loader) = &self.loader
                {
                    loader.request_front(vec![Job::ListArchive(path.clone())]);
                    self.listing_archive = Some(path);
                    self.toast("Opening archive…");
                }
            }
            None => self.toast("Nothing to open"),
        }
        self.drain_queue(event_loop);
    }
}

/// Prefer the original item or its cover. A deeper file maps to the folder
/// containing it when switching back to the hierarchical listing.
fn matching_browser_item(old: &Item, items: &[Item]) -> Option<usize> {
    let contains = |folder: &Item, file: &Item| {
        folder.container
            && !file.container
            && match (&folder.src, &file.src) {
                (Source::Path(dir), Source::Path(path)) => path.starts_with(dir),
                (
                    Source::Archive { archive, .. },
                    Source::Archive {
                        archive: other,
                        entry,
                    },
                ) => {
                    archive == other
                        && folder
                            .archive_folder
                            .as_ref()
                            .is_some_and(|f| f.contains(entry))
                }
                _ => false,
            }
    };
    items
        .iter()
        .position(|it| it.src == old.src || (old.container && old.thumb.as_ref() == Some(&it.src)))
        .or_else(|| {
            items
                .iter()
                .position(|it| contains(old, it) || contains(it, old))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_history_selection_follows_descendants_and_prefers_the_cover() {
        let file = |path: &str| Item::new(Source::Path(path.into()));
        let mut trip = file("/album/trip");
        trip.container = true;
        let images = [
            file("/album/trip2/a.png"),
            file("/album/trip/day/a.png"),
            file("/album/trip/b.png"),
        ];
        assert_eq!(matching_browser_item(&trip, &images), Some(1));
        trip.thumb = Some(images[2].src.clone());
        assert_eq!(matching_browser_item(&trip, &images), Some(2));
        assert_eq!(matching_browser_item(&images[1], &[trip]), Some(0));
    }
}
