//! wgpu renderer. Draws a `Frame`: a list of quads (some textured with an
//! image) followed by text. It knows nothing about views, themes or UI —
//! the app turns those into quads. Renders only when asked.

mod text;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use winit::window::Window;

use crate::geom::view::Rect;
use crate::model::settings::Filter;
use crate::model::theme::{Border, Color, Shadow};
use crate::ui::Align;

pub use text::Text;

pub type TexId = u64;

/// One SDF primitive instance. Matches `Inst` in quad.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    rect: [f32; 4],
    fill: [f32; 4],
    border_color: [f32; 4],
    uv0: [f32; 4],
    uv1: [f32; 4],
    params: [f32; 4],
    flags: [f32; 4],
    /// Clip rectangle (x, y, w, h); w <= 0 means none.
    clip: [f32; 4],
}

impl Quad {
    /// Only draw inside `clip` (a scrolling area); `None` leaves it as is.
    pub fn clipped(mut self, clip: Option<Rect>) -> Self {
        if let Some(c) = clip {
            self.clip = rect4(c);
        }
        self
    }
}

fn rect4(r: Rect) -> [f32; 4] {
    [r.x as f32, r.y as f32, r.w as f32, r.h as f32]
}

impl Quad {
    pub fn solid(r: Rect, fill: Color, radius: f32, border: Border, opacity: f32) -> Self {
        Self {
            rect: rect4(r),
            fill: fill.linear(),
            border_color: border.color.linear(),
            params: [radius, border.width, 0.0, opacity],
            ..Default::default()
        }
    }

    /// `s` must already be in physical px.
    pub fn shadow(r: Rect, s: &Shadow, radius: f32, opacity: f32) -> Self {
        let r = Rect::new(r.x + s.x as f64, r.y + s.y as f64, r.w, r.h);
        Self {
            rect: rect4(r),
            fill: s.color.linear(),
            params: [radius, 0.0, s.size.max(0.5), opacity],
            ..Default::default()
        }
    }

    /// A progress ring filling `progress` (0..1) clockwise from the top.
    /// `start` (turns) rotates where the arc begins — spinners animate it.
    pub fn ring(
        r: Rect,
        thickness: f32,
        progress: f32,
        start: f32,
        fill: Color,
        track: Color,
        opacity: f32,
    ) -> Self {
        Self {
            rect: rect4(r),
            fill: fill.linear(),
            border_color: track.linear(),
            params: [start, thickness, 0.0, opacity],
            flags: [0.0, 0.0, 1.0, progress.clamp(0.0, 1.0)],
            ..Default::default()
        }
    }

    pub fn image(
        r: Rect,
        uv: [[f32; 3]; 2],
        radius: f32,
        border: Border,
        checker: bool,
        opacity: f32,
    ) -> Self {
        let row = |r: [f32; 3]| [r[0], r[1], r[2], 0.0];
        Self {
            rect: rect4(r),
            fill: [0.0; 4],
            border_color: border.color.linear(),
            uv0: row(uv[0]),
            uv1: row(uv[1]),
            params: [radius, border.width, 0.0, opacity],
            flags: [1.0, checker as u32 as f32, 0.0, 0.0],
            clip: [0.0; 4],
        }
    }
}

pub struct TextDraw {
    pub text: String,
    pub family: String,
    /// Font size in physical px.
    pub size: f32,
    /// Box to align the text within (vertically centred).
    pub rect: Rect,
    pub align: Align,
    /// sRGB, alpha already multiplied by opacity.
    pub color: Color,
    /// Also clip to this (a scrolling area).
    pub clip: Option<Rect>,
}

pub struct Frame {
    pub clear: Color,
    pub checker: (Color, Color, f32),
    pub filter: Filter,
    pub quads: Vec<(Option<TexId>, Quad)>,
    /// Scene text (gallery labels, badges): drawn before the UI quads.
    pub texts: Vec<TextDraw>,
    /// Quads from this index on are UI (drawn above the scene text).
    pub ui_start: usize,
    pub ui_texts: Vec<TextDraw>,
    /// Quads from this index on are overlays (menus, tooltips), drawn after
    /// all UI text so they cover it.
    pub top_start: usize,
    pub top_texts: Vec<TextDraw>,
    /// Clip the scene (image, grid) to this rectangle; the UI is never clipped.
    pub scene_clip: Option<Rect>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    screen: [f32; 4],
    checker_a: [f32; 4],
    checker_b: [f32; 4],
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    /// Kept to rebuild the surface when it gets stuck (see `render`).
    instance: wgpu::Instance,
    window: Arc<Window>,
    /// Frames in a row the surface couldn't take.
    failures: u32,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    premultiplied: bool,

    quad_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    instances: wgpu::Buffer,
    instance_cap: usize,

    sampler_linear: wgpu::Sampler,
    sampler_nearest: wgpu::Sampler,
    mip_pipeline: wgpu::RenderPipeline,
    mip_layout: wgpu::BindGroupLayout,

    textures: HashMap<TexId, wgpu::TextureView>,
    /// Streaming textures (video): the texture itself, so frames can be
    /// written in place, and its size.
    streams: HashMap<TexId, (wgpu::Texture, (u32, u32))>,
    next_tex: TexId,
    /// 1x1 white, bound for untextured quads.
    placeholder: wgpu::TextureView,
    tex_bind_groups: HashMap<(Option<TexId>, Filter), wgpu::BindGroup>,

    pub text: Text,
}

const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl Renderer {
    pub fn new(
        instance: &wgpu::Instance,
        window: Arc<Window>,
        transparent: bool,
    ) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let surface = instance
            .create_surface(window.clone())
            .context("create surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // An image viewer has no business waking the dGPU.
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .context("no suitable GPU adapter")?;
        let info = adapter.get_info();
        log::info!("adapter: {} ({:?})", info.name, info.backend);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wlens"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            // An image viewer shares the GPU with heavier apps (games, local
            // LLMs): small memory blocks, so we start with a full GPU.
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .context("request device")?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow!("surface unsupported by adapter"))?;
        let caps = surface.get_capabilities(&adapter);
        // Prefer an sRGB surface so blending/filtering happen in linear light.
        if let Some(f) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = f;
        }
        let mut premultiplied = false;
        if transparent {
            if caps
                .alpha_modes
                .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
            {
                config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
                premultiplied = true;
            } else {
                log::warn!(
                    "transparent window requested but surface only supports {:?}",
                    caps.alpha_modes
                );
            }
        }
        // Mailbox when there is one: still in step with the display, but it
        // doesn't block waiting on the compositor while the window is hidden
        // (another workspace, screen off), which leaves some drivers (NVIDIA
        // on Wayland) stuck. Fifo is the always-available fallback.
        // $WLENS_PRESENT_MODE=fifo|mailbox|immediate overrides (troubleshooting).
        let wanted = match std::env::var("WLENS_PRESENT_MODE").as_deref() {
            Ok("fifo") => wgpu::PresentMode::Fifo,
            Ok("immediate") => wgpu::PresentMode::Immediate,
            _ => wgpu::PresentMode::Mailbox,
        };
        config.present_mode = if caps.present_modes.contains(&wanted) {
            wanted
        } else {
            wgpu::PresentMode::Fifo
        };
        log::debug!("present mode: {:?}", config.present_mode);
        surface.configure(&device, &config);

        // ---- quad pipeline
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("quad.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad"),
            bind_group_layouts: &[Some(&globals_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        const ATTRS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
        ];
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &ATTRS,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // ---- mip generation
        let mip_shader = device.create_shader_module(wgpu::include_wgsl!("mip.wgsl"));
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mip"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let mip_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mip"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mip"),
            layout: Some(&mip_pl),
            vertex: wgpu::VertexState {
                module: &mip_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mip_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: IMAGE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = |mag| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("image"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: mag,
                // Minification always filters + uses mips; `filter = nearest`
                // only changes how zoomed-in pixels look.
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let sampler_linear = sampler(wgpu::FilterMode::Linear);
        let sampler_nearest = sampler(wgpu::FilterMode::Nearest);

        let white = create_texture(&device, (1, 1), 1);
        queue.write_texture(
            white.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let placeholder = white.create_view(&Default::default());

        let instance_cap = 256;
        let instances = create_instance_buffer(&device, instance_cap);
        let text = Text::new(&device, &queue, config.format);

        Ok(Self {
            surface,
            instance: instance.clone(),
            window,
            failures: 0,
            device,
            queue,
            config,
            premultiplied,
            quad_pipeline,
            globals,
            globals_bg,
            tex_layout,
            instances,
            instance_cap,
            sampler_linear,
            sampler_nearest,
            mip_pipeline,
            mip_layout,
            textures: HashMap::new(),
            streams: HashMap::new(),
            next_tex: 1,
            placeholder,
            tex_bind_groups: HashMap::new(),
            text,
        })
    }

    pub fn max_texture_dim(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Whether the last frame(s) couldn't be drawn.
    pub fn stuck(&self) -> bool {
        self.failures > 0
    }

    /// Replace the surface with a new one for the same window: the way out
    /// when a driver's swapchain stays stuck after the window was hidden.
    pub fn rebuild_surface(&mut self) {
        match self.instance.create_surface(self.window.clone()) {
            Ok(surface) => {
                surface.configure(&self.device, &self.config);
                self.surface = surface;
                log::debug!("rebuilt the surface");
            }
            Err(e) => {
                log::warn!("couldn't rebuild the surface: {e}");
                self.surface.configure(&self.device, &self.config);
            }
        }
    }

    /// `size` must be the window's *physical* inner size.
    pub fn resize(&mut self, size: (u32, u32)) {
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        self.config.width = size.0;
        self.config.height = size.1;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn upload(&mut self, rgba: &[u8], size: (u32, u32)) -> TexId {
        let mips = 32 - size.0.max(size.1).leading_zeros();
        let texture = create_texture(&self.device, size, mips);
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.0),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        self.generate_mips(&texture, mips);
        let id = self.next_tex;
        self.next_tex += 1;
        self.textures
            .insert(id, texture.create_view(&Default::default()));
        id
    }

    #[cfg(feature = "video")]
    /// Write a video frame: reuses `id`'s texture when the size matches,
    /// else makes a new one (no mipmaps — frames change too often).
    pub fn stream(&mut self, id: Option<TexId>, rgba: &[u8], size: (u32, u32)) -> TexId {
        if let Some(id) = id
            && let Some((tex, sz)) = self.streams.get(&id)
            && *sz == size
        {
            self.queue.write_texture(
                tex.as_image_copy(),
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size.0),
                    rows_per_image: Some(size.1),
                },
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
            return id;
        }
        if let Some(old) = id {
            self.release(old);
        }
        let texture = create_texture(&self.device, size, 1);
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.0),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        let new = self.next_tex;
        self.next_tex += 1;
        self.textures
            .insert(new, texture.create_view(&Default::default()));
        self.streams.insert(new, (texture, size));
        new
    }

    pub fn release(&mut self, id: TexId) {
        self.textures.remove(&id);
        self.streams.remove(&id);
        self.tex_bind_groups.retain(|(t, _), _| *t != Some(id));
    }

    fn generate_mips(&self, texture: &wgpu::Texture, mips: u32) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        let views: Vec<_> = (0..mips)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        for pair in views.windows(2) {
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mip"),
                layout: &self.mip_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&pair[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            });
            let mut pass = begin_pass(&mut enc, &pair[1], wgpu::Color::TRANSPARENT);
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
    }

    fn ensure_tex_bind_group(&mut self, tex: Option<TexId>, filter: Filter) {
        if self.tex_bind_groups.contains_key(&(tex, filter)) {
            return;
        }
        let view = match tex {
            Some(id) => match self.textures.get(&id) {
                Some(v) => v,
                None => return,
            },
            None => &self.placeholder,
        };
        let sampler = match filter {
            Filter::Linear => &self.sampler_linear,
            Filter::Nearest => &self.sampler_nearest,
        };
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        self.tex_bind_groups.insert((tex, filter), bg);
    }

    /// Draw `frame`. False if the surface had no image to draw into (the
    /// caller should try again a little later).
    pub fn render(&mut self, frame: &Frame) -> bool {
        let surface_tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => {
                // Outdated / lost, or a timeout: the compositor stopped
                // taking frames (screen locked or off, window hidden). Some
                // drivers (NVIDIA on Wayland) never hand out images again
                // until the surface is set up again: reconfigure it, and if
                // that didn't help, build a new one.
                self.failures += 1;
                log::debug!("skipping frame: {other:?} ({} in a row)", self.failures);
                if self.failures >= 2 {
                    self.rebuild_surface();
                } else {
                    self.surface.configure(&self.device, &self.config);
                }
                return false;
            }
        };
        self.failures = 0;

        let (w, h) = self.size();
        let g = Globals {
            screen: [w as f32, h as f32, frame.checker.2.max(1.0), 0.0],
            checker_a: frame.checker.0.linear(),
            checker_b: frame.checker.1.linear(),
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));

        // Upload instances, growing the buffer if needed.
        if frame.quads.len() > self.instance_cap {
            self.instance_cap = frame.quads.len().next_power_of_two();
            self.instances = create_instance_buffer(&self.device, self.instance_cap);
        }
        let data: Vec<Quad> = frame.quads.iter().map(|(_, q)| *q).collect();
        if !data.is_empty() {
            self.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&data));
        }

        // Batch consecutive quads that share a texture.
        let mut batches: Vec<(Option<TexId>, std::ops::Range<u32>)> = Vec::new();
        for (i, (tex, _)) in frame.quads.iter().enumerate() {
            let i = i as u32;
            match batches.last_mut() {
                // Never let a batch straddle the scene/UI boundary.
                Some((t, r))
                    if t == tex
                        && i as usize != frame.ui_start
                        && i as usize != frame.top_start =>
                {
                    r.end = i + 1
                }
                _ => batches.push((*tex, i..i + 1)),
            }
        }
        for (tex, _) in &batches {
            self.ensure_tex_bind_group(*tex, frame.filter);
        }

        self.text
            .prepare(0, &self.device, &self.queue, (w, h), &frame.texts);
        self.text
            .prepare(1, &self.device, &self.queue, (w, h), &frame.ui_texts);
        self.text
            .prepare(2, &self.device, &self.queue, (w, h), &frame.top_texts);

        let target = surface_tex.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            // Window background, premultiplied (alpha only matters if transparent).
            let [r, g, b, a] = frame.clear.linear();
            let a = if self.premultiplied { a } else { 1.0 };
            let clear = wgpu::Color {
                r: (r * a) as f64,
                g: (g * a) as f64,
                b: (b * a) as f64,
                a: a as f64,
            };
            let mut pass = begin_pass(&mut enc, &target, clear);
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            // Scene quads, scene text, then UI quads and UI text on top.
            let (split, top) = (frame.ui_start as u32, frame.top_start as u32);
            let clip = frame.scene_clip.and_then(|r| {
                let x0 = r.x.max(0.0).floor() as u32;
                let y0 = r.y.max(0.0).floor() as u32;
                let x1 = ((r.x + r.w).ceil() as u32).min(w);
                let y1 = ((r.y + r.h).ceil() as u32).min(h);
                (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
            });
            for layer in 0..3 {
                match (layer, clip) {
                    (0, Some((x, y, cw, ch))) => pass.set_scissor_rect(x, y, cw, ch),
                    (1, Some(_)) => pass.set_scissor_rect(0, 0, w, h),
                    (2, _) => {}
                    _ => {}
                }
                if layer >= 1 {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_bind_group(0, &self.globals_bg, &[]);
                    pass.set_vertex_buffer(0, self.instances.slice(..));
                }
                for (tex, range) in batches.iter().filter(|(_, r)| {
                    let l = if r.start >= top {
                        2
                    } else if r.start >= split {
                        1
                    } else {
                        0
                    };
                    l == layer
                }) {
                    // A texture released mid-transition just skips its draw.
                    if let Some(bg) = self.tex_bind_groups.get(&(*tex, frame.filter)) {
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw(0..6, range.clone());
                    }
                }
                self.text.render(layer, &mut pass);
            }
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(surface_tex);
        self.text.end_frame();
        true
    }
}

fn create_instance_buffer(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quads"),
        size: (cap * std::mem::size_of::<Quad>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_texture(device: &wgpu::Device, size: (u32, u32), mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: IMAGE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn begin_pass<'a>(
    enc: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    clear: wgpu::Color,
) -> wgpu::RenderPass<'a> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
