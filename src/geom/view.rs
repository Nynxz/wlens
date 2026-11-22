//! Pure view geometry: zoom, pan, rotation, flip. No GPU, no window.
//!
//! All screen quantities are **physical pixels** (the wgpu surface size).
//! `zoom == 1.0` means one image pixel maps to exactly one physical pixel,
//! regardless of the compositor's fractional scale — that's what keeps
//! "actual size" pixel-sharp on a 1.5x display.
//!
//! The image lives in a *viewport*: the window minus any UI that reserves
//! space (e.g. a status bar). Fitting additionally honours a margin.

use crate::model::settings::{FitMode, PanMode};

/// Axis-aligned rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.1 >= self.y && p.0 < self.x + self.w && p.1 < self.y + self.h
    }
    pub fn shrink(&self, m: f64) -> Self {
        let m = m.min(self.w / 2.0 - 1.0).min(self.h / 2.0 - 1.0).max(0.0);
        Self::new(self.x + m, self.y + m, self.w - 2.0 * m, self.h - 2.0 * m)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub zoom: f64,
    /// Offset of the image centre from the viewport centre, screen px.
    pub offset: (f64, f64),
    /// Clockwise quarter turns, 0..4.
    pub quarter_turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    /// While true the view re-fits on window resize. Any manual zoom/pan clears it.
    pub fitted: bool,
    /// Drag limits; the app copies this from settings.
    pub pan: PanMode,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: (0.0, 0.0),
            quarter_turns: 0,
            flip_h: false,
            flip_v: false,
            fitted: true,
            pan: PanMode::default(),
        }
    }
}

pub const MIN_ZOOM: f64 = 0.01;
pub const MAX_ZOOM: f64 = 100.0;

impl View {
    /// Image size after rotation, before zoom.
    pub fn rotated_size(&self, img: (u32, u32)) -> (f64, f64) {
        let (w, h) = (img.0 as f64, img.1 as f64);
        if self.quarter_turns % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    pub fn fit_zoom(&self, mode: FitMode, img: (u32, u32), area: Rect) -> f64 {
        let (iw, ih) = self.rotated_size(img);
        let (ww, wh) = (area.w, area.h);
        if iw <= 0.0 || ih <= 0.0 || ww <= 0.0 || wh <= 0.0 {
            return 1.0;
        }
        let contain = (ww / iw).min(wh / ih);
        match mode {
            FitMode::None => 1.0,
            FitMode::Shrink => contain.min(1.0),
            FitMode::Contain => contain,
            FitMode::Cover => (ww / iw).max(wh / ih),
            FitMode::Width => ww / iw,
            FitMode::Height => wh / ih,
        }
    }

    /// Fit into `vp` shrunk by `margin`, centred in `vp`.
    pub fn fit(&mut self, mode: FitMode, img: (u32, u32), vp: Rect, margin: f64) {
        self.zoom = self.fit_zoom(mode, img, vp.shrink(margin));
        self.offset = (0.0, 0.0);
        self.fitted = true;
        self.clamp(img, vp);
    }

    /// Zoom to `new` keeping the image point under `anchor` (screen px) fixed.
    pub fn zoom_at(&mut self, new: f64, anchor: (f64, f64), img: (u32, u32), vp: Rect) {
        let new = new.clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = new / self.zoom;
        let c = vp.center();
        let rel = (anchor.0 - c.0, anchor.1 - c.1);
        self.offset = (
            rel.0 - (rel.0 - self.offset.0) * ratio,
            rel.1 - (rel.1 - self.offset.1) * ratio,
        );
        self.zoom = new;
        self.fitted = false;
        self.clamp(img, vp);
    }

    pub fn pan(&mut self, dx: f64, dy: f64, img: (u32, u32), vp: Rect) {
        self.offset.0 += dx;
        self.offset.1 += dy;
        self.fitted = false;
        self.clamp(img, vp);
    }

    /// Map the inner 80% of the viewport to the whole image's pan range.
    pub fn pan_to_pointer(&mut self, point: (f64, f64), img: (u32, u32), vp: Rect) {
        let (w, h) = self.rotated_size(img);
        let overflow = (
            (w * self.zoom - vp.w).max(0.0),
            (h * self.zoom - vp.h).max(0.0),
        );
        if vp.w <= 0.0 || vp.h <= 0.0 || overflow == (0.0, 0.0) {
            return;
        }
        let (cx, cy) = vp.center();
        let axis =
            |delta: f64, span: f64, excess: f64| (-delta / (0.8 * span)).clamp(-0.5, 0.5) * excess;
        self.offset = (
            axis(point.0 - cx, vp.w, overflow.0),
            axis(point.1 - cy, vp.h, overflow.1),
        );
        self.fitted = false;
    }

    /// Limit the offset according to `self.pan`. On axes where the image is
    /// larger than the viewport every mode stops at the image edges; they
    /// differ for images smaller than the viewport.
    pub fn clamp(&mut self, img: (u32, u32), vp: Rect) {
        let (iw, ih) = self.rotated_size(img);
        let mode = self.pan;
        let axis = |off: f64, size: f64, win: f64| {
            let slack = (size - win) / 2.0;
            let limit = match mode {
                _ if slack > 0.0 => slack,
                PanMode::Center => 0.0,
                // smaller image: may move until it touches the viewport edge
                PanMode::Bounded => -slack,
                // keep at least a sliver (48px, or the whole image if tinier) visible
                PanMode::Free => (win + size) / 2.0 - size.min(48.0),
            };
            off.clamp(-limit, limit)
        };
        self.offset.0 = axis(self.offset.0, iw * self.zoom, vp.w);
        self.offset.1 = axis(self.offset.1, ih * self.zoom, vp.h);
    }

    pub fn rotate(&mut self, quarter_turns: i32) {
        self.quarter_turns = (self.quarter_turns as i32 + quarter_turns).rem_euclid(4) as u8;
    }

    /// Where the (rotated) image lands on screen.
    pub fn screen_rect(&self, img: (u32, u32), vp: Rect) -> Rect {
        let (iw, ih) = self.rotated_size(img);
        let (w, h) = (iw * self.zoom, ih * self.zoom);
        let c = vp.center();
        Rect::new(
            c.0 + self.offset.0 - w / 2.0,
            c.1 + self.offset.1 - h / 2.0,
            w,
            h,
        )
    }

    /// Affine map from screen pixel position to image UV (0..1), as two rows
    /// of a 2x3 matrix: `uv = row · (x, y, 1)`. The shader does the rest.
    pub fn screen_to_uv(&self, img: (u32, u32), vp: Rect) -> [[f32; 3]; 2] {
        let (w, h) = (img.0 as f64, img.1 as f64);
        let vc = vp.center();
        let (cx, cy) = (vc.0 + self.offset.0, vc.1 + self.offset.1);
        let s = 1.0 / self.zoom;

        // Inverse rotation (y-down coords, so this undoes a clockwise turn).
        let (a, b, c, d) = match self.quarter_turns % 4 {
            0 => (1.0, 0.0, 0.0, 1.0),
            1 => (0.0, 1.0, -1.0, 0.0),
            2 => (-1.0, 0.0, 0.0, -1.0),
            _ => (0.0, -1.0, 1.0, 0.0),
        };
        // Flips happen in image space, before rotation; so undo them after.
        let fx = if self.flip_h { -1.0 } else { 1.0 };
        let fy = if self.flip_v { -1.0 } else { 1.0 };

        let row = |m0: f64, m1: f64, f: f64, size: f64| -> [f32; 3] {
            let k = f * s / size;
            [
                (k * m0) as f32,
                (k * m1) as f32,
                (0.5 - k * (m0 * cx + m1 * cy)) as f32,
            ]
        };
        [row(a, b, fx, w), row(c, d, fy, h)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uv(m: [[f32; 3]; 2], x: f64, y: f64) -> (f64, f64) {
        let f = |r: [f32; 3]| r[0] as f64 * x + r[1] as f64 * y + r[2] as f64;
        (f(m[0]), f(m[1]))
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    fn vp(w: f64, h: f64) -> Rect {
        Rect::new(0.0, 0.0, w, h)
    }

    #[test]
    fn identity_maps_centred() {
        let v = View::default();
        let m = v.screen_to_uv((100, 50), vp(200.0, 100.0));
        assert!(close(uv(m, 50.0, 25.0), (0.0, 0.0)));
        assert!(close(uv(m, 150.0, 75.0), (1.0, 1.0)));
        assert_eq!(
            v.screen_rect((100, 50), vp(200.0, 100.0)),
            Rect::new(50.0, 25.0, 100.0, 50.0)
        );
    }

    #[test]
    fn rotate_clockwise() {
        let mut v = View::default();
        v.rotate(1);
        let m = v.screen_to_uv((100, 50), vp(200.0, 200.0));
        // Image top-left corner ends up at top-right after a clockwise turn.
        assert!(close(uv(m, 125.0, 50.0), (0.0, 0.0)));
        assert!(close(uv(m, 75.0, 150.0), (1.0, 1.0)));
    }

    #[test]
    fn fit_modes_and_margin() {
        let v = View::default();
        let a = vp(200.0, 200.0);
        assert_eq!(v.fit_zoom(FitMode::Contain, (400, 200), a), 0.5);
        assert_eq!(v.fit_zoom(FitMode::Cover, (400, 200), a), 1.0);
        assert_eq!(v.fit_zoom(FitMode::Shrink, (100, 100), a), 1.0);
        assert_eq!(v.fit_zoom(FitMode::Contain, (100, 100), a), 2.0);
        let mut v = View::default();
        v.fit(FitMode::Contain, (100, 100), a, 50.0);
        assert_eq!(v.zoom, 1.0);
    }

    #[test]
    fn offset_viewport_centres_in_viewport() {
        let v = View::default();
        let area = Rect::new(0.0, 0.0, 200.0, 170.0); // 30px bar reserved at bottom
        assert_eq!(v.screen_rect((100, 100), area).center(), (100.0, 85.0));
    }

    #[test]
    fn zoom_keeps_anchor() {
        let mut v = View::default();
        let (img, area) = ((1000, 1000), vp(200.0, 200.0));
        let before = uv(v.screen_to_uv(img, area), 150.0, 60.0);
        v.zoom_at(2.0, (150.0, 60.0), img, area);
        let after = uv(v.screen_to_uv(img, area), 150.0, 60.0);
        assert!(close(before, after));
    }

    #[test]
    fn absolute_pan_maps_pointer_to_zoomed_image() {
        let area = Rect::new(100.0, 50.0, 400.0, 200.0);
        let img = (400, 300);
        let mut v = View {
            zoom: 2.0,
            ..View::default()
        };
        for (point, offset) in [
            ((140.0, 70.0), (200.0, 200.0)),
            ((460.0, 230.0), (-200.0, -200.0)),
            ((900.0, -5.0), (-200.0, 200.0)),
            ((380.0, 110.0), (-100.0, 100.0)),
            (area.center(), (0.0, 0.0)),
        ] {
            v.pan_to_pointer(point, img, area);
            assert!(close(v.offset, offset));
        }
        v.rotate(1);
        v.pan_to_pointer((460.0, 70.0), (500, 100), area);
        assert!(close(v.offset, (0.0, 400.0)));
        v.fit(FitMode::Contain, img, area, 0.0);
        let fitted = v;
        v.pan_to_pointer((420.0, 75.0), img, area);
        assert_eq!(v, fitted);
    }

    #[test]
    fn pan_modes_for_small_images() {
        let (img, area) = ((100, 100), vp(400.0, 400.0));
        let mut v = View {
            pan: PanMode::Center,
            ..View::default()
        };
        v.pan(500.0, 0.0, img, area);
        assert_eq!(v.offset.0, 0.0);
        v.pan = PanMode::Bounded;
        v.pan(500.0, 0.0, img, area);
        assert_eq!(v.offset.0, 150.0, "stops at the window edge");
        v.pan = PanMode::Free;
        v.pan(500.0, 0.0, img, area);
        assert_eq!(v.offset.0, 250.0 - 48.0, "48px stays visible");
        // large images clamp to their edges in every mode
        let mut big = View {
            pan: PanMode::Free,
            zoom: 8.0,
            ..View::default()
        };
        big.pan(10_000.0, 0.0, img, area);
        assert_eq!(big.offset.0, (800.0 - 400.0) / 2.0);
    }
}
