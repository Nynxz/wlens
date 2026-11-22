//! Thumbnail grid geometry. Pure math, no GPU: given the viewport, cell size
//! and a scroll offset, where does each item go and which are visible?

use std::ops::Range;

use crate::geom::view::Rect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub vp: Rect,
    pub cols: usize,
    /// Square image area of a cell, physical px.
    pub cell: f64,
    /// Extra height under each cell for a label (0 when labels are off).
    pub label: f64,
    pub gap: f64,
    pub pad: f64,
    /// Extra space above the first / below the last row (floating controls
    /// the grid scrolls under, but that the first/last row must clear).
    pub inset_top: f64,
    pub inset_bottom: f64,
    /// Left edge of the first column (the grid is centred horizontally).
    left: f64,
}

impl Grid {
    pub fn new(vp: Rect, cell: f64, gap: f64, pad: f64, label: f64) -> Self {
        let avail = (vp.w - 2.0 * pad).max(cell);
        let cols = (((avail + gap) / (cell + gap)).floor() as usize).max(1);
        let used = cols as f64 * cell + (cols - 1) as f64 * gap;
        Self {
            vp,
            cols,
            cell,
            label,
            gap,
            pad,
            inset_top: 0.0,
            inset_bottom: 0.0,
            left: vp.x + (vp.w - used) / 2.0,
        }
    }

    pub fn with_insets(mut self, top: f64, bottom: f64) -> Self {
        self.inset_top = top.max(0.0);
        self.inset_bottom = bottom.max(0.0);
        self
    }

    fn row_h(&self) -> f64 {
        self.cell + self.label + self.gap
    }

    pub fn rows(&self, n: usize) -> usize {
        n.div_ceil(self.cols)
    }

    /// The image area of item `i` on screen at `scroll`.
    pub fn rect(&self, i: usize, scroll: f64) -> Rect {
        let (row, col) = (i / self.cols, i % self.cols);
        Rect::new(
            self.left + col as f64 * (self.cell + self.gap),
            self.vp.y + self.pad + self.inset_top + row as f64 * self.row_h() - scroll,
            self.cell,
            self.cell,
        )
    }

    /// Cell including its label strip.
    pub fn cell_rect(&self, i: usize, scroll: f64) -> Rect {
        let r = self.rect(i, scroll);
        Rect::new(r.x, r.y, r.w, r.h + self.label)
    }

    pub fn max_scroll(&self, n: usize) -> f64 {
        let rows = self.rows(n);
        let content =
            2.0 * self.pad + self.inset_top + self.inset_bottom + rows as f64 * self.row_h()
                - if rows > 0 { self.gap } else { 0.0 };
        (content - self.vp.h).max(0.0)
    }

    /// Items at least partly on screen, plus `extra_rows` either side.
    pub fn visible(&self, n: usize, scroll: f64, extra_rows: usize) -> Range<usize> {
        let off = self.pad + self.inset_top;
        let first = ((scroll - off) / self.row_h()).floor().max(0.0) as usize;
        let last = ((scroll + self.vp.h - off) / self.row_h()).ceil().max(0.0) as usize;
        let first = first.saturating_sub(extra_rows) * self.cols;
        let last = ((last + extra_rows) * self.cols).min(n);
        first.min(last)..last
    }

    pub fn index_at(&self, p: (f64, f64), scroll: f64, n: usize) -> Option<usize> {
        if !self.vp.contains(p) {
            return None;
        }
        let range = self.visible(n, scroll, 0);
        range
            .into_iter()
            .find(|&i| self.cell_rect(i, scroll).contains(p))
    }

    /// The closest real tile, including when the pointer falls in a gutter
    /// or past the end of a short final row. The list must be nonempty.
    pub fn nearest_tile(&self, point: (f64, f64), scroll: f64, count: usize) -> usize {
        let first = self.rect(0, scroll);
        let col = ((point.0 - first.x + self.gap / 2.0) / (self.cell + self.gap)).floor() as usize;
        let row = ((point.1 - first.y + self.gap / 2.0) / self.row_h()).floor() as usize;
        (row * self.cols + col.min(self.cols - 1)).min(count - 1)
    }

    /// Keep the same vertical point of a tile at screen_y after the grid reflows.
    pub fn resized_scroll(&self, resized: &Self, index: usize, scroll: f64, screen_y: f64) -> f64 {
        let before = self.cell_rect(index, scroll);
        let after = resized.cell_rect(index, 0.0);
        let fraction = (screen_y - before.y) / before.h;
        after.y + fraction * after.h - screen_y
    }

    /// The scroll offset that brings item `i` fully into view with the least movement.
    pub fn scroll_to_show(&self, i: usize, scroll: f64, n: usize) -> f64 {
        let r = self.cell_rect(i, 0.0);
        let top = r.y - self.vp.y - self.pad - self.inset_top;
        let bottom = r.y + r.h - self.vp.y + self.pad + self.inset_bottom - self.vp.h;
        // A view shorter than one row: show the row's top.
        let bottom = bottom.min(top);
        scroll
            .clamp(bottom, top)
            .clamp(0.0, self.max_scroll(n).max(0.0))
    }

    /// Rows that fit in one screen, for page up/down.
    pub fn page_rows(&self) -> usize {
        ((self.vp.h - 2.0 * self.pad) / self.row_h())
            .floor()
            .max(1.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        // 4 columns of 100 with 10 gap fit in 460 - 2*10 padding = 440 (+10 gap)
        Grid::new(Rect::new(0.0, 0.0, 460.0, 300.0), 100.0, 10.0, 10.0, 0.0)
    }

    #[test]
    fn columns_and_rects() {
        let g = grid();
        assert_eq!(g.cols, 4);
        // 4*100 + 3*10 = 430 wide, centred in 460 => starts at 15
        assert_eq!(g.rect(0, 0.0), Rect::new(15.0, 10.0, 100.0, 100.0));
        assert_eq!(g.rect(5, 0.0), Rect::new(125.0, 120.0, 100.0, 100.0));
        assert_eq!(g.rect(5, 50.0).y, 70.0);
    }

    #[test]
    fn scrolling() {
        let g = grid();
        // 10 items = 3 rows: 10 + 3*110 - 10 + 10 = 340 content, 300 viewport
        assert_eq!(g.max_scroll(10), 40.0);
        assert_eq!(g.visible(10, 0.0, 0), 0..10);
        assert_eq!(
            g.scroll_to_show(9, 0.0, 10),
            40.0,
            "last row scrolls into view"
        );
        assert_eq!(g.scroll_to_show(0, 40.0, 10), 0.0, "first row scrolls back");
        assert_eq!(
            g.scroll_to_show(4, 20.0, 10),
            20.0,
            "visible item: no movement"
        );
    }

    #[test]
    fn resizing_deep_in_a_grid_preserves_the_pointer_row() {
        let grid = grid();
        let larger = Grid::new(grid.vp, 144.0, 10.0, 10.0, 16.0).with_insets(20.0, 6.0);
        let pointer = (121.0, 110.0); // in the gutter
        let scroll = 3000.0;
        let anchor = grid.nearest_tile(pointer, scroll, 500);
        assert!(anchor > 100);
        let next = grid.resized_scroll(&larger, anchor, scroll, pointer.1);
        let tile = larger.cell_rect(anchor, next);
        assert!((tile.y..=tile.y + tile.h).contains(&pointer.1));
        let restored = larger.resized_scroll(&grid, anchor, next, pointer.1);
        assert!((restored - scroll).abs() < 0.001);
        assert_eq!(grid.nearest_tile(pointer, scroll, 3), 2);
    }

    #[test]
    fn hit_testing() {
        let g = grid();
        assert_eq!(g.index_at((20.0, 15.0), 0.0, 10), Some(0));
        assert_eq!(
            g.index_at((120.0, 15.0), 0.0, 10),
            None,
            "gap between cells"
        );
        assert_eq!(
            g.index_at((350.0, 250.0), 0.0, 10),
            None,
            "past the last item"
        );
    }

    #[test]
    fn tiny_viewport_does_not_panic() {
        // A viewport shorter than one cell (e.g. while the layout settles).
        let g = Grid::new(Rect::new(0.0, 0.0, 300.0, 40.0), 160.0, 8.0, 16.0, 0.0);
        let s = g.scroll_to_show(5, 0.0, 20);
        assert!(s.is_finite() && s >= 0.0);
    }
}
