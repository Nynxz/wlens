//! Moving through items: navigation, list changes from Lua, levels.

use super::*;

impl App {
    /// Replace or extend the item list (from `wlens.set_items` / `add_items`).
    pub(super) fn apply_items(&mut self, items: Vec<Item>, append: bool, start: Option<usize>) {
        let before = self.items.len();
        if append {
            self.items.extend(items);
        } else {
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
            ItemsOp::Set(list, start) => self.apply_items(list, false, start),
            ItemsOp::Add(list) => self.apply_items(list, true, None),
            ItemsOp::Push(list, start, title) => self.push_level(list, start, title),
            ItemsOp::Pop => {
                self.back();
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
        self.stack.push(Level {
            items: std::mem::take(&mut self.items),
            index: self.index,
            scroll: self.gallery.scroll,
            title: self.level_title.take(),
            more: std::mem::take(&mut self.script.shared.borrow_mut().more),
        });
        self.level_title = title;
        self.gallery.scroll = 0.0;
        self.gallery.active = true;
        self.apply_items(items, false, start);
        self.gallery.shown = self.gallery.scroll;
        self.script.emit("select");
        self.script.emit("level");
    }

    /// Go back up a level. Returns false at the top.
    pub(super) fn back(&mut self) -> bool {
        let Some(level) = self.stack.pop() else {
            return false;
        };
        self.items = level.items;
        self.index = level.index.min(self.items.len().saturating_sub(1));
        self.level_title = level.title;
        {
            let mut sh = self.script.shared.borrow_mut();
            sh.items = self.items.clone();
            sh.more = level.more;
        }
        self.pending_advance = false;
        self.gallery.active = true;
        self.gallery.scroll = level.scroll;
        self.gallery.shown = level.scroll;
        self.gallery.requested = None;
        self.sync();
        self.script.emit("select");
        self.script.emit("level");
        self.redraw();
        true
    }

    /// Enter the current container, or view the current image.
    pub(super) fn open_current(&mut self, event_loop: &ActiveEventLoop) {
        let Some(item) = self.items.get(self.index).cloned() else {
            return;
        };
        if !item.container {
            return self.set_gallery(false);
        }
        match item.open {
            Some(Action::Command(c)) => self.exec(event_loop, c),
            Some(Action::Lua(f)) => {
                let info = self.script.info_value();
                self.script.call_with(&f, info, "item open");
            }
            None => self.toast("Nothing to open"),
        }
        self.drain_queue(event_loop);
    }
}
