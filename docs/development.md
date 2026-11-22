# Development

```sh
cargo test                 # unit tests (geometry, layout, parsing, themes, caches)
cargo clippy --all-targets
cargo build --release      # target/release/wlens
```

## Source layout

| | |
|---|---|
| `main.rs`, `files.rs` | Command line and startup; turning arguments into items |
| `app/` | The viewer. `mod.rs` holds the state; the rest is split by job: `view` (showing the current image, animation), `nav` (navigation, item lists, levels), `gallery` (grid, thumbnails), `commands`, `frame` (per-frame work), `input` (mouse), `events` (the winit loop), `script` (Lua glue), `ipc` (answering socket requests), `video` (playback), `cache` |
| `model/` | Plain data and parsing: `command` (keep [configuration.md](configuration.md) in sync), `keys`, `settings`, `theme` (incl. colour parsing), `item` |
| `geom/` | Pure maths: `view` (fit, zoom, pan, rotation → UVs), `grid` (gallery layout), `anim` (easing) |
| `ui/` | Lua-declared UI: `parse` (Lua → nodes), `eval` (live values), `tree` (rebuilt children), `layout`, `interact` (visibility, hover, hit-testing), `draw` |
| `io/` | Worker threads and files: `loader` (decoding incl. animations), `thumbs` (freedesktop cache), `remote` (HTTP), `video` (GStreamer), `state` (remembered options), `atomic` (safe writes), `notify` (desktop notification when startup fails), `ipc` (the control socket) |
| `render/` | wgpu: one instanced SDF shader for boxes/images, glyphon text in two layers (scene, UI), GPU mipmaps |
| `lua/` | The Lua host (`mod.rs`), the API (`api.rs`), item info (`info.rs`), plus `default.lua`, `widgets.lua`, the presets in `themes/` and the palettes in `palettes/` |

## Design rules

- **Everything is a command.** Keys, Lua, menus, buttons and the IPC socket
  all go through `command.rs`, so new features are reachable from all of them
  at once.
- **Lua on one thread.** Lua never touches viewer state directly: it edits
  settings/UI or queues commands and list changes, applied after it returns.
- **Physical pixels.** The surface is the window's physical size; zoom 100%
  is one image pixel per screen pixel, so images stay sharp at fractional
  scales. Theme lengths are logical and scaled at layout time.
- **Render on demand.** Frames are drawn only while something changes or
  animates; idle CPU should be zero. Mouse motion redraws only when a hover
  target changes.
- **Work off the main thread.** Decoding, thumbnailing and HTTP run on
  workers; queues are re-prioritised on navigation so stale work is dropped.
- **Wayland only.** winit is built without X11.

## Testing tips

- Point the caches somewhere disposable so tests don't touch yours:
  `XDG_CACHE_HOME=/tmp/wlens-test wlens …`.
- Drive the viewer from a throwaway config: `wlens.on("start", …)` with
  `wlens.after(…)` steps, `wlens.exec(…)`, and `print(…)` of
  `wlens.current()` / `wlens.items()`; finish with `wlens.quit()`.
- Test windows take keyboard focus on most compositors; keep them short.
