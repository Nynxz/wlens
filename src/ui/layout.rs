//! Measuring and placing nodes: a small flexbox (row/column, gap, padding,
//! grow, justify), anchors, reserved edges, tooltip placement.

use super::*;

impl Ui {
    /// Space reserved at the window edges (top, right, bottom, left), physical px.
    pub fn reserved(&self, win: Rect, gallery: bool) -> [f64; 4] {
        let mut r = [0.0f64; 4];
        for n in &self.nodes {
            if n.reserve == Reserve::No || n.rect.w <= 0.0 {
                continue;
            }
            let active = match n.reserve {
                Reserve::No => false,
                Reserve::Always => n.vis >= 0.5,
                Reserve::Gallery => gallery && n.show != Show::Never,
            };
            if !active {
                continue;
            }
            let b = n.rect;
            match n.anchor {
                (_, Align::Start) => r[0] = r[0].max(b.y + b.h - win.y),
                (_, Align::End) => r[2] = r[2].max(win.y + win.h - b.y),
                (Align::Start, _) => r[3] = r[3].max(b.x + b.w - win.x),
                (Align::End, _) => r[1] = r[1].max(win.x + win.w - b.x),
                _ => {}
            }
        }
        r
    }

    pub fn layout(&mut self, win: Rect, scale: f32, theme: &Theme, m: &mut dyn Measure) {
        self.win = win;
        for n in &mut self.nodes {
            n.fit_width = None;
            n.squeezed_out = false;
        }
        self.squeeze.clear();
        self.squeeze_hide.clear();
        self.place_all(win, scale, theme, m);
        // Rows that didn't fit asked some text to shorten and some items to
        // hide: measure with that and lay out again.
        if !self.squeeze.is_empty() || !self.squeeze_hide.is_empty() {
            for (i, w) in std::mem::take(&mut self.squeeze) {
                self.nodes[i].fit_width = Some(w);
            }
            for i in std::mem::take(&mut self.squeeze_hide) {
                self.nodes[i].squeezed_out = true;
            }
            self.place_all(win, scale, theme, m);
            self.squeeze.clear();
            self.squeeze_hide.clear();
        }
        self.finish_layout(win, scale, theme, m);
    }

    fn place_all(&mut self, win: Rect, scale: f32, theme: &Theme, m: &mut dyn Measure) {
        // Children always have larger indices than parents, so a reverse
        // sweep measures leaves first.
        for i in (0..self.nodes.len()).rev() {
            self.measure(i, scale, theme, m);
        }
        for r in self.roots.clone() {
            let n = &self.nodes[r];
            let w = resolve(n.width, n.measured.0, win.w as f32, scale);
            let h = cap_height(
                n,
                resolve(n.height, n.measured.1, win.h as f32, scale),
                win.h as f32,
                scale,
            );
            let place = |a: Align, off: f32, size: f32, start: f64, avail: f64| -> f64 {
                let off = (off * scale) as f64;
                match a {
                    Align::Start => start + off,
                    Align::Center => start + (avail - size as f64) / 2.0 + off,
                    Align::End => start + avail - size as f64 - off,
                }
            };
            let x = place(n.anchor.0, n.x, w, win.x, win.w);
            let y = place(n.anchor.1, n.y, h, win.y, win.h);
            self.arrange(r, Rect::new(x, y, w as f64, h as f64), scale);
        }
    }

    fn finish_layout(&mut self, win: Rect, scale: f32, theme: &Theme, m: &mut dyn Measure) {
        // Clip rectangles from scrolling ancestors.
        // Attached elements (dropdowns) move next to their target.
        let gap = (6.0 * scale) as f64;
        for i in 0..self.nodes.len() {
            let Some(target) = self.nodes[i].attach.clone() else {
                continue;
            };
            let r = self.nodes[i].rect;
            if r.w <= 0.0 {
                continue;
            }
            let Some(t) = self
                .nodes
                .iter()
                .find(|n| n.id.as_deref() == Some(target.as_str()) && n.rect.w > 0.0)
                .map(|n| n.rect)
            else {
                continue;
            };
            let x = match self.nodes[i].attach_align {
                Align::Start => t.x,
                Align::Center => t.x + (t.w - r.w) / 2.0,
                Align::End => t.x + t.w - r.w,
            };
            let x = x.min(win.x + win.w - r.w).max(win.x);
            let above = t.y - win.y;
            let below = win.y + win.h - (t.y + t.h);
            let y = if above >= r.h + gap || above > below {
                t.y - gap - r.h
            } else {
                t.y + t.h + gap
            };
            let y = y.min(win.y + win.h - r.h).max(win.y);
            self.arrange(i, Rect::new(x, y, r.w, r.h), scale);
        }
        // Attached panels now have their final position, including their scroll clip.
        for r in self.roots.clone() {
            self.assign_clip(r, None, scale);
        }
        if let Some((i, _)) = self.tip {
            let n = &self.nodes[i];
            let (tw, th) = m.text_size(
                &n.tooltip_value,
                &theme.font_family,
                theme.tooltip_size * scale,
            );
            let (px, py, gap) = (
                (10.0 * scale) as f64,
                (5.0 * scale) as f64,
                (8.0 * scale) as f64,
            );
            let w = (tw as f64 + 2.0 * px).min(win.w);
            let h = th as f64 + 2.0 * py;
            let r = n.rect;
            let mut y = r.y - gap - h;
            if y < win.y {
                y = r.y + r.h + gap;
            }
            let spare = win.w - w;
            let margin = gap.min(spare / 2.0);
            let centered = r.center().0 - win.x - w / 2.0;
            let x = win.x + centered.clamp(margin, spare - margin);
            self.tip_rect = Rect::new(x, y, w, h);
        }
    }

    pub(super) fn measure(&mut self, i: usize, scale: f32, theme: &Theme, m: &mut dyn Measure) {
        let n = &self.nodes[i];
        let [pt, pr, pb, pl] = n.padding.map(|p| p * scale);
        let mut content = (0.0f32, 0.0f32);
        let mut caret = 0.0;
        if (n.text.is_some() || n.input.is_some()) && !n.text_value.is_empty() {
            let family = n.family(theme);
            let size = n.font_size.unwrap_or(theme.font_size) * scale;
            content = m.text_size(&n.text_value, &family, size);
            caret = if n.input_placeholder { 0.0 } else { content.0 };
            self.nodes[i].display_text = self.nodes[i].text_value.clone();
            let n = &self.nodes[i];
            let limit = match (n.max_width.map(|w| w * scale), n.fit_width) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if let Some(max) = limit
                && content.0 > max
            {
                let short = shorten(m, &n.text_value, &family, size, max, n.truncate_end);
                content = m.text_size(&short, &family, size);
                self.nodes[i].display_text = short;
            }
        }
        let n = &self.nodes[i];
        let gap = n.gap * scale;
        let mut count = 0;
        for &c in &n.children {
            if self.collapsed(c) || self.nodes[c].floating {
                continue;
            }
            let cn = &self.nodes[c];
            let cw = match cn.width {
                Size::Px(p) => p * scale,
                Size::Percent(_) => 0.0,
                Size::Auto => cn.measured.0,
            };
            let ch = match cn.height {
                Size::Px(p) => p * scale,
                Size::Percent(_) => 0.0,
                Size::Auto => cn.measured.1,
            };
            match n.dir {
                Dir::Row => content = (content.0 + cw, content.1.max(ch)),
                Dir::Column => content = (content.0.max(cw), content.1 + ch),
            }
            count += 1;
        }
        if count > 1 {
            match n.dir {
                Dir::Row => content.0 += gap * (count - 1) as f32,
                Dir::Column => content.1 += gap * (count - 1) as f32,
            }
        }
        // A wrapping row of known width: lines of children.
        if n.wrap
            && n.dir == Dir::Row
            && let Size::Px(w) = n.width
        {
            let avail = (w * scale - pl - pr).max(1.0);
            let (mut x, mut line_h, mut total_h, mut max_w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut first = true;
            for &c in &n.children {
                if self.collapsed(c) || self.nodes[c].floating {
                    continue;
                }
                let cn = &self.nodes[c];
                let cw = match cn.width {
                    Size::Px(p) => p * scale,
                    _ => cn.measured.0,
                };
                let ch = match cn.height {
                    Size::Px(p) => p * scale,
                    _ => cn.measured.1,
                };
                if !first && x + gap + cw > avail {
                    total_h += line_h + gap;
                    max_w = max_w.max(x);
                    x = cw;
                    line_h = ch;
                } else {
                    x += if first { cw } else { gap + cw };
                    line_h = line_h.max(ch);
                }
                first = false;
            }
            content = (max_w.max(x), total_h + line_h);
        }
        self.nodes[i].caret = caret;
        self.nodes[i].measured = (content.0 + pl + pr, content.1 + pt + pb);
    }

    fn assign_clip(&mut self, i: usize, clip: Option<Rect>, scale: f32) {
        self.nodes[i].clip = clip;
        let n = &self.nodes[i];
        let inner_clip = if n.scroll {
            let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
            let r = n.rect;
            let mine = Rect::new(
                r.x + pl,
                r.y + pt,
                (r.w - pl - pr).max(0.0),
                (r.h - pt - pb).max(0.0),
            );
            Some(match clip {
                Some(c) => {
                    let x0 = mine.x.max(c.x);
                    let y0 = mine.y.max(c.y);
                    let x1 = (mine.x + mine.w).min(c.x + c.w);
                    let y1 = (mine.y + mine.h).min(c.y + c.h);
                    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
                }
                None => mine,
            })
        } else {
            clip
        };
        for c in self.nodes[i].children.clone() {
            self.assign_clip(c, inner_clip, scale);
        }
    }

    /// What an overflowing row can give up, looking through nested rows:
    /// shrinkable text, and items with a priority.
    fn overflow_candidates(&self, c: usize, texts: &mut Vec<usize>, ranked: &mut Vec<usize>) {
        let n = &self.nodes[c];
        if self.collapsed(c) || n.floating || n.grow && n.children.is_empty() {
            return;
        }
        if n.priority.is_some() {
            ranked.push(c);
        }
        if n.text.is_some() && (n.shrink || n.max_width.is_some()) {
            texts.push(c);
        } else if n.dir == Dir::Row && !n.scroll {
            for &k in &n.children {
                self.overflow_candidates(k, texts, ranked);
            }
        }
    }

    /// A collapsed subtree takes no space and draws nothing: zero every rect
    /// in it (descendants would otherwise keep their last positions).
    fn clear_rects(&mut self, i: usize) {
        self.nodes[i].rect = Rect::default();
        for c in self.nodes[i].children.clone() {
            self.clear_rects(c);
        }
    }

    pub(super) fn arrange(&mut self, i: usize, rect: Rect, scale: f32) {
        self.nodes[i].rect = rect;
        let n = &self.nodes[i];
        let [pt, pr, pb, pl] = n.padding.map(|p| (p * scale) as f64);
        let inner = Rect::new(
            rect.x + pl,
            rect.y + pt,
            (rect.w - pl - pr).max(0.0),
            (rect.h - pt - pb).max(0.0),
        );
        let gap = (n.gap * scale) as f64;
        let (main_avail, cross_avail) = match n.dir {
            Dir::Row => (inner.w, inner.h),
            Dir::Column => (inner.h, inner.w),
        };
        let (dir, align, justify) = (n.dir, n.align, n.justify);
        let (children, gone): (Vec<usize>, Vec<usize>) = n
            .children
            .clone()
            .into_iter()
            .partition(|&c| !self.collapsed(c));
        for c in gone {
            self.clear_rects(c);
        }
        // Anchored children float over this element's inner box.
        let (floating, children): (Vec<usize>, Vec<usize>) =
            children.into_iter().partition(|&c| self.nodes[c].floating);
        for c in floating {
            let cn = &self.nodes[c];
            let w = resolve(cn.width, cn.measured.0, inner.w as f32, scale);
            let h = cap_height(
                cn,
                resolve(cn.height, cn.measured.1, inner.h as f32, scale),
                inner.h as f32,
                scale,
            );
            let place = |a: Align, off: f32, size: f32, start: f64, avail: f64| -> f64 {
                let off = (off * scale) as f64;
                match a {
                    Align::Start => start + off,
                    Align::Center => start + (avail - size as f64) / 2.0 + off,
                    Align::End => start + avail - size as f64 - off,
                }
            };
            // Too big for its area (a small window)? Keep it on screen.
            let win = self.win;
            let x = place(cn.anchor.0, cn.x, w, inner.x, inner.w)
                .min(win.x + win.w - w as f64)
                .max(win.x);
            let y = place(cn.anchor.1, cn.y, h, inner.y, inner.h)
                .min(win.y + win.h - h as f64)
                .max(win.y);
            self.arrange(c, Rect::new(x, y, w as f64, h as f64), scale);
        }
        if children.is_empty() {
            return;
        }

        let main_of = |c: &Node| -> (f64, f64) {
            let (size, measured) = match dir {
                Dir::Row => (c.width, c.measured.0),
                Dir::Column => (c.height, c.measured.1),
            };
            let cross = match dir {
                Dir::Row => resolve(c.height, c.measured.1, cross_avail as f32, scale),
                Dir::Column => resolve(c.width, c.measured.0, cross_avail as f32, scale),
            } as f64;
            let main = resolve(size, measured, main_avail as f32, scale) as f64;
            // max_height caps the vertical size (main for columns, cross for rows).
            let cap = |v: f64, avail: f64| match c.max_height {
                Some(m) => v.min(resolve(m, f32::MAX, avail as f32, scale) as f64),
                None => v,
            };
            match dir {
                Dir::Column => (cap(main, main_avail), cross),
                Dir::Row => (main, cap(cross, cross_avail)),
            }
        };
        let sizes: Vec<(f64, f64)> = children.iter().map(|&c| main_of(&self.nodes[c])).collect();
        let wrap = self.nodes[i].wrap && dir == Dir::Row;
        // Scrolling is vertical: columns, and wrapping rows (lines stack down).
        let scrolls = self.nodes[i].scroll && (dir == Dir::Column || wrap);
        if wrap {
            // Lines left to right, top to bottom.
            let place = |sizes: &[(f64, f64)]| -> (Vec<(f64, f64)>, f64) {
                let (mut x, mut y, mut line_h) = (0.0f64, 0.0f64, 0.0f64);
                let mut at = Vec::with_capacity(sizes.len());
                for &(w, h) in sizes {
                    if x > 0.0 && x + w > inner.w + 0.5 {
                        x = 0.0;
                        y += line_h + gap;
                        line_h = 0.0;
                    }
                    at.push((x, y));
                    x += w + gap;
                    line_h = line_h.max(h);
                }
                (at, y + line_h)
            };
            let (at, content) = place(&sizes);
            let off = if scrolls {
                let n = &mut self.nodes[i];
                n.content_h = content + (n.rect.h - inner.h);
                n.scroll_y = n.scroll_y.clamp(0.0, (content - inner.h).max(0.0));
                n.scroll_y
            } else {
                0.0
            };
            for ((&c, &(w, h)), (x, y)) in children.iter().zip(&sizes).zip(at) {
                self.arrange(c, Rect::new(inner.x + x, inner.y + y - off, w, h), scale);
            }
            return;
        }

        // Too full? First shrinkable text gives up width (re-measured with
        // "…"), then items with a `priority` hide, lowest first. Both look
        // inside child rows too (a bar's sections), so the whole row fits.
        if dir == Dir::Row && !scrolls {
            let gaps = gap * (children.len().saturating_sub(1)) as f64;
            let fixed: f64 = children
                .iter()
                .zip(&sizes)
                .filter(|(c, _)| !self.nodes[**c].grow)
                .map(|(_, s)| s.0)
                .sum();
            let mut over = fixed + gaps - main_avail;
            if over > 0.5 {
                let (mut texts, mut ranked) = (Vec::new(), Vec::new());
                for &c in &children {
                    self.overflow_candidates(c, &mut texts, &mut ranked);
                }
                // Text keeps a comfortable width (~120px) while low-priority
                // items hide; only then does it shorten further (to 40px).
                let comfy = (120.0 * scale) as f64;
                let hard = (40.0 * scale) as f64;
                let mut widths: Vec<f64> = texts
                    .iter()
                    .map(|&c| self.nodes[c].measured.0 as f64)
                    .collect();
                let give = |widths: &mut Vec<f64>, over: &mut f64, floor: f64| {
                    let room: f64 = widths.iter().map(|w| (w - floor).max(0.0)).sum();
                    if room > 0.0 && *over > 0.5 {
                        let take = over.min(room);
                        for w in widths.iter_mut() {
                            *w -= (*w - floor).max(0.0) / room * take;
                        }
                        *over -= take;
                    }
                };
                give(&mut widths, &mut over, comfy);
                ranked.sort_by_key(|&c| self.nodes[c].priority.unwrap_or(i32::MAX));
                for c in ranked {
                    if over <= 0.5 {
                        break;
                    }
                    over -= self.nodes[c].measured.0 as f64 + gap;
                    self.squeeze_hide.push(c);
                }
                if over < -0.5 {
                    // Hiding freed more than needed: give text some width back.
                    let mut spare = -over;
                    for (k, &c) in texts.iter().enumerate() {
                        let back = (self.nodes[c].measured.0 as f64 - widths[k])
                            .max(0.0)
                            .min(spare);
                        widths[k] += back;
                        spare -= back;
                    }
                    over = -spare;
                }
                give(&mut widths, &mut over, hard);
                for (k, &c) in texts.iter().enumerate() {
                    if widths[k] + 0.5 < self.nodes[c].measured.0 as f64 {
                        let [_, pr, _, pl] = self.nodes[c].padding.map(|p| p * scale);
                        self.squeeze
                            .push((c, (widths[k] as f32 - pl - pr).max(1.0)));
                    }
                }
            }
        }
        let fixed: f64 = children
            .iter()
            .zip(&sizes)
            .filter(|(c, _)| !self.nodes[**c].grow)
            .map(|(_, s)| s.0)
            .sum();
        // In a scrolling column, children keep their natural size (no growing).
        let growers = if scrolls {
            0
        } else {
            children.iter().filter(|&&c| self.nodes[c].grow).count()
        };
        let gaps = gap * (children.len() - 1) as f64;
        let leftover = (main_avail - fixed - gaps).max(0.0);
        let cursor = match (growers, justify) {
            (0, Align::Center) => leftover / 2.0,
            (0, Align::End) => leftover,
            _ => 0.0,
        };
        let scroll_y = if scrolls {
            // Keep the offset within the content (it may have shrunk).
            let content = fixed + gaps;
            let n = &mut self.nodes[i];
            n.content_h = content + (n.rect.h - inner.h);
            n.scroll_y = n.scroll_y.clamp(0.0, (content - main_avail).max(0.0));
            n.scroll_y
        } else {
            0.0
        };
        let mut cursor = cursor - scroll_y;
        for (&c, &(main, cross)) in children.iter().zip(&sizes) {
            let main = if self.nodes[c].grow && growers > 0 {
                leftover / growers as f64
            } else {
                main
            };
            let cross_off = match align {
                Align::Start => 0.0,
                Align::Center => (cross_avail - cross) / 2.0,
                Align::End => cross_avail - cross,
            };
            let r = match dir {
                Dir::Row => Rect::new(inner.x + cursor, inner.y + cross_off, main, cross),
                Dir::Column => Rect::new(inner.x + cross_off, inner.y + cursor, cross, main),
            };
            self.arrange(c, r, scale);
            cursor += main + gap;
        }
    }
}

/// Apply a node's `max_height` (px or % of `avail`).
fn cap_height(n: &Node, h: f32, avail: f32, scale: f32) -> f32 {
    match n.max_height {
        Some(m) => h.min(resolve(m, f32::MAX, avail, scale)),
        None => h,
    }
}

pub(super) fn resolve(size: Size, measured: f32, avail: f32, scale: f32) -> f32 {
    match size {
        Size::Auto => measured,
        Size::Px(p) => p * scale,
        Size::Percent(p) => avail * p,
    }
}

/// `text` shortened with "…" to fit `max` physical px: at the end, or in
/// the middle (keeping the start and, for a file name, its extension).
pub fn shorten(
    m: &mut dyn Measure,
    text: &str,
    family: &str,
    size: f32,
    max: f32,
    end: bool,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| -> String {
        if keep >= chars.len() {
            return text.to_string();
        }
        if end {
            chars[..keep].iter().collect::<String>() + "…"
        } else {
            let tail = keep / 2;
            let head = keep - tail;
            chars[..head].iter().collect::<String>()
                + "…"
                + &chars[chars.len() - tail..].iter().collect::<String>()
        }
    };
    if m.text_size(text, family, size).0 <= max {
        return text.to_string();
    }
    // Longest prefix/suffix that fits.
    let (mut lo, mut hi) = (0, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if m.text_size(&cut(mid), family, size).0 <= max {
            lo = mid
        } else {
            hi = mid - 1
        }
    }
    cut(lo)
}
