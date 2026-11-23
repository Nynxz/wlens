//! Text via glyphon (cosmic-text shaping + a GPU glyph atlas).
//!
//! The font database is loaded lazily on the first measure, so a theme with
//! no text (the minimal preset) never pays for scanning system fonts.
//! Shaped buffers are cached by (text, family, size) and evicted when unused.

use std::collections::HashMap;

use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};

use super::TextDraw;
use crate::ui::{Align, Measure};

type Key = (String, String, u32);

struct Entry {
    buffer: Buffer,
    size: (f32, f32),
    ink_center: Option<(f32, f32)>,
    used: u64,
}

pub struct Text {
    fonts: Option<FontSystem>,
    swash: SwashCache,
    atlas: TextAtlas,
    /// One renderer per layer (scene, UI) so UI text draws above scene boxes.
    renderers: [TextRenderer; 3],
    viewport: Viewport,
    buffers: HashMap<Key, Entry>,
    frame: u64,
    has_glyphs: [bool; 3],
}

fn family(name: &str) -> Family<'_> {
    match name {
        "sans-serif" | "sans" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" | "mono" => Family::Monospace,
        other => Family::Name(other),
    }
}

impl Text {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let mut renderer =
            || TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let renderers = [renderer(), renderer(), renderer()];
        Self {
            fonts: None,
            swash: SwashCache::new(),
            atlas,
            renderers,
            viewport: Viewport::new(device, &cache),
            buffers: HashMap::new(),
            frame: 0,
            has_glyphs: [false; 3],
        }
    }

    fn entry(&mut self, text: &str, fam: &str, size: f32, center_ink: bool) -> &mut Entry {
        let key: Key = (text.to_string(), fam.to_string(), size.to_bits());
        let frame = self.frame;
        let fonts = self.fonts.get_or_insert_with(|| {
            let t = std::time::Instant::now();
            let fs = FontSystem::new();
            log::debug!("loaded {} font faces in {:?}", fs.db().len(), t.elapsed());
            fs
        });
        let e = self.buffers.entry(key).or_insert_with(|| {
            let mut buffer = Buffer::new(fonts, Metrics::new(size, (size * 1.25).ceil()));
            buffer.set_size(None, None);
            // "Family:600" carries a weight (see ui::Node::family).
            let (base, weight) = match fam.rsplit_once(':') {
                Some((b, w)) if w.parse::<u16>().is_ok() => (b, w.parse::<u16>().ok()),
                _ => (fam, None),
            };
            let mut attrs = Attrs::new().family(family(base));
            if let Some(w) = weight {
                attrs = attrs.weight(Weight(w));
            }
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(fonts, false);
            let (mut w, mut h) = (0.0f32, 0.0f32);
            for run in buffer.layout_runs() {
                w = w.max(run.line_w);
                h = h.max(run.line_top + run.line_height);
            }
            Entry {
                buffer,
                size: (w.ceil(), h.ceil()),
                ink_center: None,
                used: frame,
            }
        });
        if center_ink && e.ink_center.is_none() {
            let mut bounds: Option<(i32, i32, i32, i32)> = None;
            for run in e.buffer.layout_runs() {
                for glyph in run.glyphs {
                    let g = glyph.physical((0.0, 0.0), 1.0);
                    let Some(image) = self.swash.get_image_uncached(fonts, g.cache_key) else {
                        continue;
                    };
                    let p = image.placement;
                    if p.width == 0 || p.height == 0 {
                        continue;
                    }
                    let x = g.x + p.left;
                    let y = run.line_y.round() as i32 + g.y - p.top;
                    let right = x + p.width as i32;
                    let bottom = y + p.height as i32;
                    bounds = Some(match bounds {
                        Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(right), b.max(bottom)),
                        None => (x, y, right, bottom),
                    });
                }
            }
            e.ink_center = Some(
                bounds.map_or((e.size.0 / 2.0, e.size.1 / 2.0), |(l, t, r, b)| {
                    ((l + r) as f32 / 2.0, (t + b) as f32 / 2.0)
                }),
            );
        }
        e.used = frame;
        e
    }

    /// Prepare one layer (0 = scene, 1 = UI, 2 = overlays on top of the UI).
    pub fn prepare(
        &mut self,
        layer: usize,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: (u32, u32),
        draws: &[TextDraw],
    ) {
        self.has_glyphs[layer] = !draws.is_empty();
        if draws.is_empty() {
            return;
        }
        // Make sure every buffer exists before borrowing them all.
        let mut placed = Vec::with_capacity(draws.len());
        for d in draws {
            let entry = self.entry(&d.text, &d.family, d.size, d.center_ink);
            if d.center_ink
                && let Some((cx, cy)) = entry.ink_center
            {
                let (x, y) = d.rect.center();
                placed.push((
                    (x - f64::from(cx)).round() as f32,
                    (y - f64::from(cy)).round() as f32,
                ));
                continue;
            }
            let size = entry.size;
            let x = match d.align {
                Align::Start => d.rect.x,
                Align::Center => d.rect.x + (d.rect.w - size.0 as f64) / 2.0,
                Align::End => d.rect.x + d.rect.w - size.0 as f64,
            };
            let y = d.rect.y + (d.rect.h - size.1 as f64) / 2.0;
            placed.push((x.round() as f32, y.round() as f32));
        }
        self.viewport.update(
            queue,
            Resolution {
                width: screen.0,
                height: screen.1,
            },
        );

        let areas = draws.iter().zip(&placed).map(|(d, &(left, top))| {
            let e = &self.buffers[&(d.text.clone(), d.family.clone(), d.size.to_bits())];
            let [r, g, b, a] = d.color.0.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            TextArea {
                buffer: &e.buffer,
                left,
                top,
                scale: 1.0,
                // Clip to the element's box (with a little slack for descenders).
                bounds: {
                    let (mut l, mut t, mut r, mut b) = (
                        d.rect.x.floor(),
                        (d.rect.y - 2.0).floor(),
                        (d.rect.x + d.rect.w).ceil(),
                        (d.rect.y + d.rect.h + 2.0).ceil(),
                    );
                    if let Some(c) = d.clip {
                        l = l.max(c.x.floor());
                        t = t.max(c.y.floor());
                        r = r.min((c.x + c.w).ceil());
                        b = b.min((c.y + c.h).ceil());
                    }
                    TextBounds {
                        left: l as i32,
                        top: t as i32,
                        right: r as i32,
                        bottom: b.max(t) as i32,
                    }
                },
                default_color: glyphon::Color::rgba(r, g, b, a),
                custom_glyphs: &[],
            }
        });
        let fonts = self.fonts.as_mut().expect("fonts loaded by entry()");
        if let Err(e) = self.renderers[layer].prepare(
            device,
            queue,
            fonts,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            log::warn!("text prepare: {e}");
            self.has_glyphs[layer] = false;
        }
    }

    pub fn render(&self, layer: usize, pass: &mut wgpu::RenderPass<'_>) {
        if self.has_glyphs[layer]
            && let Err(e) = self.renderers[layer].render(&self.atlas, &self.viewport, pass)
        {
            log::warn!("text render: {e}");
        }
    }

    pub fn end_frame(&mut self) {
        let frame = self.frame;
        // Keep a few frames of slack so blinking/hover text doesn't reshape.
        self.buffers
            .retain(|_, e| frame.saturating_sub(e.used) < 120);
        self.atlas.trim();
        self.frame += 1;
    }
}

impl Measure for Text {
    fn text_size(&mut self, text: &str, family: &str, size: f32) -> (f32, f32) {
        self.entry(text, family, size, false).size
    }
}
