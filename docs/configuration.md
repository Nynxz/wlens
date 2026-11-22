# Configuration

wlens runs Lua at startup, in this order:

1. the built-in defaults ([`src/lua/default.lua`](../src/lua/default.lua)) — keybindings
   and the `simple` theme;
2. your config: `$XDG_CONFIG_HOME/wlens/init.lua`, **or** the file given with
   `-c` (which replaces `init.lua` rather than adding to it).

Errors in your config are reported (file and line) but don't stop the
viewer from opening. Errors in scripts while running (hooks, `fetch` and
`download` callbacks, buttons) show briefly as a toast, as well as in the
terminal. A complete example: [`examples/scripts/init.lua`](../examples/scripts/init.lua).

Modules you `require` are looked up next to the running script first, then
in `~/.config/wlens/`, so you can split a config into files
(`require("comfyui")` loads `comfyui.lua` from either place).

A script can also be a **folder**: `wlens -c myscript/` runs
`myscript/init.lua`, and its other files are `require`-able by name
(`require("sidebar")` → `myscript/sidebar.lua`). `wlens.script_dir` is the
folder, for loading non-Lua files next to it.

## Options

Behaviour options, set from Lua or with the `set` command:

```lua
wlens.opt.fit = "contain"     -- or wlens.set("fit", "contain"), or `set fit contain`
print(wlens.opt.wrap)
```

| Key | Values | Default | |
|---|---|---|---|
| `fit` | `none` `shrink` `contain` `cover` `width` `height` | `shrink` | How images fit the window |
| `filter` | `linear` `nearest` | `linear` | How zoomed-in pixels look |
| `zoom_step` | number > 1 | `1.25` | Factor per zoom in/out |
| `wrap` | bool | `false` | Loop past the first/last image |
| `pan` | `center` `bounded` `free` | `bounded` | Small images: fixed in the centre / draggable within the window / anywhere |
| `preload` | number | `2` | Images decoded ahead and behind |
| `cache_mb` | number | `512` | Decoded-image cache budget |
| `proxy` | URL | (none) | HTTP / SOCKS proxy for everything wlens fetches, e.g. `http://127.0.0.1:8118`; `$WLENS_PROXY` sets it too, else the `*_PROXY` environment variables apply |
| `remote_cache_mb` | number | `2048` | Size limit of the download cache (`~/.cache/wlens/remote`); least recently used files are removed at startup |
| `video_end` | `loop` `next` `stop` | `loop` | What happens when a video ends |

Theme properties (dotted keys like `image.radius`) use the same `set`; see
[Theming](theming.md).

**Remembered between runs:** options and theme properties you change while
wlens is running — from the menu, a key, or a `set …` command — are saved
(per config / script, in `~/.local/state/wlens/state.json`) and re-applied
next time, on top of what your config sets. Settings your config file sets
itself aren't saved: the config stays the default. So if you change
something from the menu and later edit the same option in `init.lua`, the
remembered value still wins: change it back in the menu, or delete the file
(or the script's entry) to start fresh. `wlens.theme { … }` sets values
without remembering them, for a script's own defaults.

## Keybindings

```lua
wlens.bind("x", "rotate 90")                         -- a command
wlens.bind("C-w", function() wlens.spawn('swww img "$WLENS_FILE"') end, "Set wallpaper") -- a Lua function + description
wlens.bind("x", nil)                                  -- remove
wlens.bind("j", "gallery down", "Down", "gallery")    -- only in the gallery
wlens.unbind_all()                                    -- start from nothing
```

The description is what the help overlay (`?`) and menus show.

**Key syntax:** modifier prefixes `C-` (Ctrl), `A-` (Alt), `S-` (Shift),
`M-` (Super), then a key: a character (`n`, `N`, `+`, `?`) or a name
(`Left`, `Return`, `space`, `Escape`, `BackSpace`, `Home`, `F1`…). Character
keys match the character typed, so `N` already means Shift+n.

**Mouse names:** `ScrollUp`, `ScrollDown`, `ClickImage`, `ClickBackdrop`,
`DoubleClick`, `RightClick`, `MiddleClick`, `MouseBack`, `MouseForward` (the
side buttons; `MouseBack` goes back by default). Drag with the left or middle
button to pan. A `MiddleClick` binding runs on release after a click.

**Modes:** a binding with a 4th argument (`"image"`, `"video"` or
`"gallery"`) only applies in that mode and wins over the global one. Video
mode falls back to image bindings, so zoom and navigation still work.

## Commands

Everything wlens does is a command. As a string, a command is the quickest
way to bind a key or give a button an action (`wlens.bind("x", "rotate 90")`).
In Lua, almost every one is also a function of the same name, with the words
as arguments (the exceptions: `ui show ID` is `wlens.show(ID)` and so on,
since `wlens.ui` builds the UI; `set` is `wlens.set(key, value)`; `goto` is
`wlens.go`):

```lua
wlens.next(4)                 -- next 4
wlens.go(12)                  -- goto 12  (`goto` is a Lua keyword)
wlens.zoom("in")              -- zoom in
wlens.video("seek", "+10")    -- video seek +10
wlens.toggle("help")          -- ui toggle help  (also show, hide, focus, blur)
wlens.exec("rotate 90")       -- any command string
```

| Command | |
|---|---|
| `next [n]`, `prev [n]`, `first`, `last`, `goto N` | Navigate (1-based `goto`) |
| `zoom in\|out\|actual\|150%\|2x` | Zoom |
| `fit [mode]` | Fit (optionally switching mode) |
| `pan DX DY` | Pan by logical pixels |
| `rotate ±90\|180`, `flip h\|v`, `reset` | Transform |
| `set KEY VALUE` | Set an option or theme property (`toggle` flips booleans) |
| `gallery` (or `grid`) `toggle\|open\|close\|up\|down\|left\|right\|pageup\|pagedown\|bigger\|smaller\|scroll N` | The grid |
| `open` | Open the current item (enter a container, or view an image) |
| `back` | Image → its grid → parent level |
| `refresh` | Fire the `refresh` hook (or reload the image) |
| `ui show\|hide\|toggle ID` | Show/hide a UI element by id |
| `ui focus\|blur ID` | Start / stop typing in a text field |
| `copy path\|name\|dir\|image` | Copy to the clipboard (needs `wl-clipboard`) |
| `animation play\|pause\|toggle` | Animated GIF / WebP / APNG |
| `video play\|pause\|toggle`, `video seek 30\|+5\|-5\|50%`, `video volume 80\|+10\|-10`, `video mute` | Video playback |
| `fullscreen [on\|off\|toggle]`, `quit` | |
| `reload` | Reload the current image from disk |
| `lua CODE` | Run Lua |
