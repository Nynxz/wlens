//! Event loop glue: owns the window, renderer, loader, script host and the
//! viewer state; turns input into `Command`s and state into `Frame`s.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{CursorIcon, Fullscreen, Window, WindowId};

use crate::geom::anim::{approach, lerp};
use crate::geom::grid::Grid;
use crate::geom::view::{Rect, View};
use crate::io::loader::{Decoded, Job, LoadResult, Loader, ThumbResult};
use crate::lua::{Action, ImageInfo, ItemsOp, Script};
use crate::model::command::{Command, Zoom};
use crate::model::item::{Item, Source};
use crate::model::keys::Chord;
use crate::model::theme::{Border, Color, TransitionKind};
use crate::render::{Frame, Quad, Renderer, TexId, TextDraw};
use crate::ui::{self, Align, Ui};

mod cache;
mod commands;
mod events;
mod frame;
mod gallery;
mod input;
mod ipc;
mod nav;
mod script;
#[cfg(feature = "video")]
mod video;
mod view;

use cache::{Cache, Slot};

pub enum UserEvent {
    Loaded {
        src: Source,
        result: LoadResult,
    },
    Thumb {
        src: Source,
        result: ThumbResult,
    },
    /// From a video player (`id` = which one, so stale players are ignored).
    /// A video URL fetched into the cache (or found too big: `None`, stream it).
    #[cfg(feature = "video")]
    VideoCached {
        src: Source,
        path: Option<std::path::PathBuf>,
    },
    #[cfg(feature = "video")]
    Video {
        id: u64,
        event: crate::io::video::VideoEvent,
    },
    /// A `wlens.download` reporting progress (bytes, total) or finishing.
    Download {
        id: u64,
        progress: Option<(u64, Option<u64>)>,
        done: Option<Result<u64, String>>,
    },
    Fetched {
        id: u64,
        result: crate::io::remote::FetchResult,
    },
    /// A request from the IPC socket; answer it on `reply`.
    Ipc {
        request: String,
        reply: std::sync::mpsc::Sender<String>,
    },
}

/// An image on the GPU and the (animated) view it's drawn with.
struct Layer {
    src: Source,
    /// The frame being shown.
    tex: TexId,
    size: (u32, u32),
    file_size: u64,
    view: View,
    /// Frames of an animated image (texture, delay); empty for stills.
    frames: Vec<(TexId, u32)>,
    frame: usize,
    /// When the next frame is due.
    frame_due: Instant,
}

impl Layer {
    /// Every texture this layer owns (all frames, or the one still).
    fn release(self, r: &mut Renderer) {
        if self.frames.is_empty() {
            r.release(self.tex);
        }
        for (t, _) in self.frames {
            r.release(t);
        }
    }
}

struct Transition {
    from: Option<Layer>,
    start: Instant,
    /// +1 when moving forward through the list, -1 backwards.
    dir: f32,
}

struct Press {
    button: MouseButton,
    mods: ModifiersState,
    at: PhysicalPosition<f64>,
    dragged: bool,
    /// Pressed on a slider: drags set its value instead of panning.
    slider: Option<usize>,
    /// The press started on a UI control that consumes drags.
    blocks_drag: bool,
}

struct ThumbTex {
    tex: TexId,
    size: (u32, u32),
    orig: Option<(u32, u32)>,
    level: u32,
    /// Frame number it was last drawn on, for eviction.
    used: u64,
}

/// At most this many thumbnails live on the GPU (~256 KiB each at 256px).
const MAX_GPU_THUMBS: usize = 600;

/// A level we can go `back` to (see `wlens.push_items`).
struct Level {
    items: Vec<Item>,
    index: usize,
    scroll: f64,
    title: Option<String>,
    more: bool,
}

#[derive(Default)]
struct Gallery {
    active: bool,
    /// 0 = image view, 1 = grid; animated for the cross-fade.
    t: f64,
    /// Scroll target and the animated value drawn.
    scroll: f64,
    shown: f64,
    thumbs: HashMap<Source, ThumbTex>,
    pending: HashSet<(Source, u32)>,
    failed: HashSet<Source>,
    frame: u64,
    /// What we last asked the loader for, to avoid re-queueing every frame.
    requested: Option<(Range<usize>, u32, usize)>,
    /// Tile labels shortened to fit: (name, width px, size) -> label.
    labels: HashMap<(String, u32, u32), String>,
}

pub struct App {
    items: Vec<Item>,
    index: usize,
    start_fullscreen: bool,
    script: Script,
    proxy: EventLoopProxy<UserEvent>,
    instance: wgpu::Instance,

    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    loader: Option<Loader>,
    cache: Cache,

    /// What's on screen; `current.view` glides towards `target`.
    current: Option<Layer>,
    target: View,
    transition: Option<Transition>,
    nav_dir: f32,
    gallery: Gallery,
    stack: Vec<Level>,
    level_title: Option<String>,
    /// (depth, item count) for which `end_reached` already fired.
    end_fired: Option<(usize, usize)>,
    /// Tried to step past the end while more was loading: step once it arrives.
    pending_advance: bool,
    /// Animated images are paused (`animation pause`).
    anim_paused: bool,
    #[cfg(feature = "video")]
    video: Option<video::VideoState>,
    #[cfg(feature = "video")]
    next_video_id: u64,
    /// A video URL being fetched into the cache before playing.
    #[cfg(feature = "video")]
    video_pending: Option<Source>,
    /// Video volume (1.0 = 100%) and mute, kept across videos.
    volume: f64,
    muted: bool,
    /// Set by commands that should open the current container once they finish.
    pending_open: bool,
    /// The window or renderer couldn't be created; exit with an error.
    pub startup_failed: bool,

    ui: Ui,
    text_dirty: bool,
    scale: f32,
    /// Window minus space reserved by UI.
    viewport: Rect,
    /// Scene drawing is clipped to this (the main view region), if any.
    scene_clip: Option<Rect>,
    /// Extra top/bottom padding for the grid (see `layout_ui`).
    grid_insets: (f64, f64),
    /// The text input with keyboard focus (by element id).
    focus: Option<String>,

    mods: ModifiersState,
    cursor: Option<PhysicalPosition<f64>>,
    last_motion: Instant,
    press: Option<Press>,
    last_click: Option<(Instant, PhysicalPosition<f64>)>,
    /// Anchor for the next zoom command (cursor for mouse zoom, else centre).
    zoom_anchor: Option<(f64, f64)>,
    scroll_accum: f64,
    last_frame: Instant,
    /// A frame the surface couldn't take: when to try again, and how many
    /// failed in a row (the wait grows, so a hidden window doesn't keep
    /// blocking the main thread).
    frame_retry: Option<Instant>,
    frame_failures: u32,
    animating: bool,
    /// (UI element, thumbnail) under the cursor, to redraw only on change.
    last_hover: (Option<usize>, Option<usize>),
}

impl App {
    pub fn new(
        items: Vec<Item>,
        index: usize,
        start_fullscreen: bool,
        script: Script,
        proxy: EventLoopProxy<UserEvent>,
        instance: wgpu::Instance,
    ) -> Self {
        let ui = script
            .shared
            .borrow_mut()
            .pending_ui
            .take()
            .unwrap_or_default();
        script.shared.borrow_mut().items = items.clone();
        Self {
            index: index.min(items.len().saturating_sub(1)),
            items,
            start_fullscreen,
            script,
            proxy,
            instance,
            window: None,
            renderer: None,
            loader: None,
            cache: Cache::default(),
            current: None,
            target: View::default(),
            transition: None,
            nav_dir: 1.0,
            gallery: Gallery::default(),
            stack: Vec::new(),
            level_title: None,
            end_fired: None,
            pending_advance: false,
            anim_paused: false,
            #[cfg(feature = "video")]
            video: None,
            #[cfg(feature = "video")]
            next_video_id: 0,
            #[cfg(feature = "video")]
            video_pending: None,
            // Remembered across runs (and scripts): see video_command.
            volume: crate::io::state::section("_global", "player")
                .get("volume")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
                .clamp(0.0, 1.5),
            muted: crate::io::state::section("_global", "player")
                .get("muted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            pending_open: false,
            startup_failed: false,
            ui,
            text_dirty: true,
            scale: 1.0,
            viewport: Rect::new(0.0, 0.0, 1.0, 1.0),
            scene_clip: None,
            grid_insets: (0.0, 0.0),
            focus: None,
            mods: ModifiersState::empty(),
            cursor: None,
            last_motion: Instant::now(),
            press: None,
            last_click: None,
            zoom_anchor: None,
            scroll_accum: 0.0,
            last_frame: Instant::now(),
            frame_retry: None,
            frame_failures: 0,
            animating: false,
            last_hover: (None, None),
        }
    }

    // ------------------------------------------------------------ helpers

    fn img_size(&self) -> (u32, u32) {
        self.current.as_ref().map_or((1, 1), |l| l.size)
    }

    fn margin(&self) -> f64 {
        (self.script.shared.borrow().settings.theme.image_margin * self.scale) as f64
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn current_src(&self) -> Option<&Source> {
        self.items.get(self.index).map(|i| &i.src)
    }

    /// The current item is a video we can play.
    fn current_is_video(&self) -> bool {
        self.items.get(self.index).is_some_and(|i| i.is_video())
    }

    fn mode(&self) -> &'static str {
        if self.gallery.active {
            "gallery"
        } else if self.current_is_video() {
            "video"
        } else {
            "image"
        }
    }

    fn toast(&self, msg: &str) {
        self.script.call_api("toast", msg);
    }

    /// Copy the pan mode from settings into the view before moving it.
    fn sync_pan_mode(&mut self) {
        self.target.pan = self.script.shared.borrow().settings.pan;
    }

    // --------------------------------------------------------- navigation

    // ------------------------------------------------------------ levels

    // ----------------------------------------------------------- gallery

    // ----------------------------------------------------------- commands

    // ----------------------------------------------------------------- UI

    // ----------------------------------------------------------- frames

    // ------------------------------------------------------------- mouse
}
