# Usage

```
wlens [OPTIONS] [FILES|DIRS|URLS|-]...
```

With no arguments wlens opens the images in the current directory. Passing a
directory opens its folder browser. Use `-r` for a flat recursive image list. `-`
reads paths or URLs from stdin, one per line. `http(s)://` arguments are
downloaded in the background and cached.

## Viewing images in archives

Opening `wlens photos.zip` shows its top-level folders and images. An archive
with images only at the root opens on the first image. Use `--browse` to start
in the grid in either case. Open a folder with Return or a click. A lone
wrapper folder with the same name as the archive opens automatically.

`wlens --flat photos.zip` reads images from every subfolder as one naturally
sorted sequence. `--flat --browse` shows the sequence as thumbnails. Press
`v` or change **Settings → Gallery → Flat view** to switch while browsing.
The choice applies when you go Back or Forward and is remembered for the next launch.
Passing `--flat` enables it for that launch.
`wlens -n 12 photos.zip` also uses the flat sequence to start on page 12.
ZIP, RAR, 7z, tar and common compressed tar formats are supported, including
the CBZ, CBR, CB7 and CBT extensions.

When you first view an image or request its thumbnail, a background worker
extracts the images in archive order. Solid archives need just one
decompression pass. Extracted images are stored under
`$XDG_CACHE_HOME/wlens/archives` (normally `~/.cache/wlens/archives`) with
hashed names. Later sessions reuse them. Editing or replacing an archive
creates a new cache directory. You can remove the archives cache while
wlens is closed to reclaim space.

An archive can have up to 10,000 entries, 256 MiB per image and 2 GiB of
uncompressed content in total. Archive pages use the viewer's supported
image formats and must be unencrypted. Each page is limited to 64 megapixels
(64 × 1024² pixels) and 256 MiB of decoded pixel data before display or thumbnailing.

## Flags

| Flag | |
|---|---|
| `-r, --recursive` | Include images in subdirectories |
| `-S, --siblings` | Given one file, browse the other images in its folder |
| `--flat` | Read media from every subfolder as one list |
| `-b, --browse` | Folders and archives become tiles you can open (see [Galleries](galleries.md)) |
| `-n, --start N` | Start at the Nth image (1-based) |
| `-f, --fullscreen` | Start fullscreen |
| `-t, --theme NAME` | `minimal`, `simple`, `lightbox`, or your own; overrides `wlens.use()` in configs. Env: `WLENS_THEME` |
| `-p, --palette NAME` | Colour palette: a palette theme name (`gruvbox`, `gruvbox:light`) or a `.lua`/`.json` path. Env: `WLENS_PALETTE` |
| `--list-palettes` | List the palettes: the built-in ones (`gruvbox`, `tokyo-night`, `strawberry-milk`) and yours in `~/.config/wlens/themes/` |
| `-c, --config FILE` | Use this config **instead of** `init.lua` |
| `--no-config` | Built-in defaults only |
| `--msg COMMAND…` | Send a command to a running wlens and exit (see below) |
| `--socket PATH` | Which viewer `--msg` talks to |

`WLENS_APP_ID` sets the window's app id (its `class` in Hyprland; default
`wlens`), so a config can get its own window rules, e.g.
`WLENS_APP_ID=wlens-lightbox wlens --theme lightbox` with a float rule for
`wlens-lightbox`.

## Controlling a running viewer

Each viewer listens on a Unix socket, so scripts and compositor keybinds can
drive it:

```sh
wlens --msg next              # any command from the command language
wlens --msg zoom in
wlens --msg gallery toggle
wlens --msg get current       # the current item as JSON (as wlens.current())
```

`--msg` talks to `$WLENS_SOCKET` if set, else the most recently started
viewer; `--socket PATH` picks one. Programs started by wlens (`wlens.spawn`)
get `$WLENS_SOCKET`, so they control the viewer that started them. A failed
command prints the error and exits with status 1.

The socket is `$XDG_RUNTIME_DIR/wlens/<pid>.sock`, readable only by you. To
talk to it directly, write one request per line and read one JSON line back
for each: `{"ok":true}`, `{"ok":true,"data":{…}}` for `get current`, or
`{"ok":false,"error":"…"}`. The [command language](configuration.md#commands)
lists every command.

## Keys

Press `?` (or `F1`) for the full, live list — it reflects your own bindings.
The defaults:

| Keys | Action |
|---|---|
| `n` `space` `→` `l` / `p` `Backspace` `←` `h` | Next / previous |
| `Ctrl+n` / `Ctrl+p` | Forward / back 10 |
| `g` `Home` / `G` `End` | First / last |
| `+` `=` `-`, scroll wheel | Zoom (around the cursor) |
| `1` `2` `3` | 100% / 200% / 300% |
| `f` `w` `W` `e` `E` `c` | Fit: re-fit, shrink, contain, width, height, cover |
| drag, `H J K L`, `Shift+arrows` | Pan |
| `>` `<` `\|` `_` `r` | Rotate, flip, reset |
| double-click | Toggle fit / 100% |
| `Return` | Gallery (thumbnail grid) |
| `m` | Menu |
| `,` | Settings (fit, smoothing, checkerboard, wrap, transitions, thumbnails, video end…) |
| `b` | Hide / show the bars and sidebars (the image gets the whole window) |
| `y` / `Y` | Copy path / copy image |
| `s` | Slideshow |
| `a` / `A` | Smooth ↔ pixelated / checkerboard |
| `F` | Fullscreen |
| `P` | Play / pause an animated image |
| right-click | Return to thumbnails, then up one folder |
| `v` | Toggle the remembered flat-view setting |
| mouse back button | Back (image → grid → parent level) |
| `F5` | Refresh: asks a script to reload its list (its `refresh` hook), else reloads the image |
| `R` | Reload the image from disk |
| `Esc` | Close menu/help → go up a level → quit |
| `q` | Quit |

Volume and mute are remembered between runs. For videos: `space` / click play/pause, `Shift+←` `Shift+→` seek
5s (`Ctrl+←` `Ctrl+→` 30s), `↑` `↓` volume, `M` mute, `0`–`9` jump to 0–90%.
Arrows and the click zones still go to the previous/next item. The seek and
volume bars can be clicked or dragged.

In the gallery the arrows (and `h j k l`) move the selection, `PageUp` /
`PageDown` move a screen, `Return` or `space` opens,
`Backspace` goes back a level, `+`/`-` or `Ctrl+scroll` resize thumbnails and
`c` switches thumbnails between crop and fit.

## Mouse

- Click the left or right part of the window to go back/forward (invisible
  click zones; the cursor changes over them when there's somewhere to go).
- Drag to pan. Small images can be moved within the window (`pan = "bounded"`).
- Scroll to zoom around the cursor; in the gallery, scroll or drag to move.
- Click the file name in the bar to copy its path.
- Clicking empty space closes an open menu; it never quits.

## Troubleshooting

**The window stops updating after switching workspaces or idling.** wlens
uses the Mailbox present mode when the GPU offers it and rebuilds its surface
when frames stop arriving. If a driver still gets stuck, try another mode:
`WLENS_PRESENT_MODE=fifo` (or `immediate`). `RUST_LOG=wlens=debug` logs the
mode in use and any skipped frames.

**Nothing opens when started from a launcher.** If the window or renderer
can't be created, wlens logs why; when it has no terminal to log to, it also
sends a desktop notification (via `notify-send`), e.g. when the GPU is out of
memory.
