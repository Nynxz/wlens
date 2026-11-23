# Theming

Three layers, from coarse to fine:

1. **A preset** sets the overall look and builds the UI.
2. **A palette** colours it.
3. **Theme properties** adjust anything individually.

```lua
wlens.use("lightbox", { palette = "tokyo-night", transition = "slide" })
wlens.theme { image = { radius = 4 }, gallery = { size = 200 } }
```

Per run: `wlens --theme lightbox --palette gruvbox`. `--theme` overrides
every `wlens.use()` call (including in example scripts) and keeps that
call's options; `--palette` colours whichever theme is used unless it picked
its own.

## Presets

| | |
|---|---|
| `minimal` | Just the image, and the clean starting point for your own layout. Options: `background`, `checker`, `font`, `palette` |
| `simple` (default) | A status bar that reserves space. Options: `palette`, `position` (`"top"`), `accent`, `foreground`, `background`, `bar_background`, `bar_height`, `font`, `font_size`, `smooth`, `transition`, `bar = false`, `layout`, `nav`, `nav_zones = false`, `help_button = false` |
| `lightbox` | Image floating on a translucent, blurred backdrop with hover controls. Goes fullscreen when it starts (`fullscreen = false` keeps the window). Options: `palette`, `backdrop`, `margin`, `radius`, `transition`, `easing`, `fit`, `font`, `nav`, `nav_zones = false`, `fullscreen = false`, `help_button = false` |

The `simple` bar can be rearranged with `layout`; see [UI](ui.md).

The presets' buttons have ids — `menu_button`, `back_button`,
`refresh_button`, `help_button` — so a script can hide one
without rebuilding the theme: `wlens.hide("refresh_button")`.

How much the lightbox blurs what's behind it is up to the compositor
(Hyprland: `decoration:blur`); wlens only asks for a transparent window.

## Palettes

A palette is a colour set any preset can use. Three are built in: `gruvbox`,
`tokyo-night` and `strawberry-milk` (light, on `lightbox`, so it goes
fullscreen too). Your own are **palette themes**: Lua files in
`~/.config/wlens/themes/` that return a table of colours instead of a
function. The built-in ones in [`src/lua/palettes/`](../src/lua/palettes) are
a good start: copy one there (keeping its name overrides the built-in one).

```lua
-- ~/.config/wlens/themes/mine.lua
return {
  name = "Mine",
  base = "simple",            -- the preset `wlens --theme mine` uses (default simple)
  dark = {
    background = "#1d2021", foreground = "#ebdbb2",
    card = "#282828", popover = "#282828", sidebar = "#1d2021",
    primary = "#fe8019", accent = "#3c3836", border = "#504945",
    muted = "#3c3836", muted_foreground = "#a89984",
    radius = "0.3rem", font_sans = "'Inter', sans-serif",
  },
  light = { ... },            -- optional
}
```

Use it as a theme (`--theme mine`, `wlens.use("mine")`) or to colour another
preset (`--palette mine`, `--palette mine:light`,
`wlens.use("lightbox", { palette = "mine" })`). `wlens --list-palettes` shows
every palette you can use. A palette can also be a path to a `.lua` file, a
`.json` file of the same shape (`{ "name": …, "dark": {…}, "light": {…} }`, or
just the colours), or a table given inline. Keys can be written plain (`muted_foreground`)
or CSS-style (`--muted-foreground`).

| Key | Used for |
|---|---|
| `background` / `foreground` | Window, text |
| `card` | Gallery tiles, bar (unless `sidebar`) |
| `sidebar` | Bar background |
| `popover` (+ `popover_foreground`) | Menus, help, toasts, tooltips |
| `primary`, `ring` | Accent, gallery selection |
| `accent` | Hover highlight |
| `border` | Outlines, gallery hover |
| `muted`, `muted_foreground` | Checkerboard, secondary text |
| `radius` | Corner rounding everywhere (`rem` = 16px) |
| `font_sans` | Font (first family) |

## Colours

Anywhere a colour is accepted: `#rgb`, `#rrggbb`, `#rrggbbaa`,
`rgb(255 128 0)`, `rgba(255, 0, 0, 0.5)`, `oklch(0.7 0.12 30)`,
`oklch(0.7 0.12 30 / 50%)`, `transparent`. `wlens.color(c, alpha)` returns a
normalised `#rrggbbaa`, optionally with a new alpha.

## Theme properties

Set with `wlens.theme { group = { key = value } }` (nested tables flatten),
`wlens.opt["group.key"] = value`, or the command `set group.key value`. Lists
(like the checker colours) only go through `wlens.theme`.
Lengths are logical pixels.

| Key | |
|---|---|
| `window.background` | Colour; alpha makes the window translucent (needs `window.transparent`) |
| `window.transparent` | Ask the compositor for transparency (read at startup) |
| `image.margin`, `image.radius` | Space around a fitted image; corner rounding |
| `image.border.width`, `image.border.color` | |
| `image.shadow.size`, `.color`, `.x`, `.y` | Drop shadow (size 0 = none) |
| `image.checker` | Checkerboard behind transparency: on/off, or two colours (`wlens.theme { image = { checker = { "#313244", "#45475a" } } }`) |
| `image.checker.a`, `image.checker.b`, `image.checker.size` | The two colours one at a time; square size |
| `transition.kind` | `none` `fade` `slide` `zoom` |
| `transition.duration`, `transition.easing` | Seconds; `linear` `ease_in` `ease_out` `ease_in_out` `ease_out_back` `ease_out_expo` |
| `anim.view` | Seconds for zoom/pan to glide (0 = instant) |
| `anim.ui` | Seconds for UI fades and hover |
| `ui.idle` | Seconds before hover-revealed controls hide |
| `font.family`, `font.size`, `font.color` | |
| `tooltip.background`, `.color`, `.radius`, `.size`, `.delay` | |
| `gallery.size`, `.gap`, `.padding`, `.radius` | Thumbnail cell geometry |
| `gallery.fit` | `cover` (crop) or `contain` (letterbox) |
| `gallery.background`, `.selected`, `.selected_width`, `.hover` | Cell colours |
| `gallery.labels`, `.label_color`, `.label_size` | Names under thumbnails |
| `gallery.anim` | Seconds for the grid fade and scroll glide |

## Writing a theme

A theme is a Lua function of an options table. Put it in
`~/.config/wlens/themes/NAME.lua` and use it with `wlens.use("NAME")` or
`--theme NAME`:

```lua
return function(o)
  local w = wlens.widgets
  wlens.use("minimal", { palette = o.palette })  -- a clean slate (in a palette's colours)
  wlens.theme { image = { radius = 6 } }
  wlens.ui(w.list(
    w.layout {                          -- regions around the image/grid
      bottom = w.bar { left = { w.filename() }, right = { w.counter() } },
      right = w.slot("right", { dir = "column", background = w.style.bar_background }),
    },
    w.nav_zones(),
    w.menu(), w.help(), w.toast()       -- floating
  ))
end
```

Key bindings and hooks a theme registers (`wlens.bind`, `wlens.on`) belong
to it: applying another theme (or the same one again) removes them first, so
nothing doubles up or lingers.

The built-in presets in [`src/lua/themes/`](../src/lua/themes) are written exactly
this way.
