//! Present a bottom-anchored UI on Hyprland without stretching its fixed edge.
//! Hyprland can crop or repeat trailing texels while it waits for a buffer
//! matching the new window geometry. During height changes we store the
//! bottom row first, keeping both axes aligned with the window.

use std::sync::Arc;

use wayland_client::{
    Connection, Proxy,
    backend::{Backend, ObjectId},
    protocol::{wl_output::Transform, wl_surface::WlSurface},
};
use winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle},
    window::Window,
};

/// winit sets a viewport destination in logical units on fractional outputs.
/// An integer buffer scale lets Hyprland resize that viewport along the axis
/// being dragged. Both attachment dimensions must be multiples of the scale.
pub(super) fn resize_buffer_scale((w, h): (u32, u32), output_scale: f64, limit: u32) -> u32 {
    let scale = output_scale.ceil() as u32;
    if scale <= 1 || output_scale.fract() == 0.0 {
        return 1;
    }
    let largest = w.max(h);
    if largest
        .checked_next_multiple_of(scale)
        .is_some_and(|n| n <= limit)
    {
        scale
    } else {
        1
    }
}

pub(super) struct WindowSurface {
    surface: WlSurface,
    _connection: Connection,
    _window: Arc<Window>,
}

impl WindowSurface {
    pub fn new(window: Arc<Window>) -> Option<Self> {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
        if !desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("Hyprland"))
        {
            return None;
        }
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: both handles come from this window. Its Arc outlives the
        // borrowed surface and display connection, including during drop.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }
                .ok()?;
        let connection = Connection::from_backend(backend);
        let surface = WlSurface::from_id(&connection, id).ok()?;
        (surface.version() >= 3).then_some(Self {
            surface,
            _connection: connection,
            _window: window,
        })
    }

    pub fn set_transform(&self, transform: Transform) {
        self.surface.set_buffer_transform(transform);
    }

    pub fn set_scale(&self, scale: u32) {
        self.surface.set_buffer_scale(scale as i32);
    }
}

/// Reuse a little spare capacity as the pointer crosses adjacent sizes. The
/// render viewport and the final copy both use the exact current pixel size.
pub(super) struct ResizeTarget {
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

impl ResizeTarget {
    pub fn fits(&self, w: u32, h: u32) -> bool {
        self.size.0 >= w
            && self.size.1 >= h
            && self.size.0 / 2 <= w.max(64)
            && self.size.1 / 2 <= h.max(64)
    }

    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        (w, h): (u32, u32),
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let capacity = |n: u32| {
            n.next_multiple_of(64)
                .min(device.limits().max_texture_dimension_2d)
        };
        let size = (capacity(w), capacity(h));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("resize frame"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resize frame"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            view,
            bind_group,
            size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resize_buffer_scale;

    #[test]
    fn fractional_resize_buffer_fits_both_dimensions() {
        assert_eq!(resize_buffer_scale((871, 451), 1.5, 8192), 2);
        assert_eq!(resize_buffer_scale((871, 451), 1.0, 8192), 1);
        assert_eq!(resize_buffer_scale((451, 8192), 2.25, 8192), 1);
    }
}
