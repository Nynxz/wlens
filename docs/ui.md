# UI and widgets

The on-screen UI is a list of elements given to `wlens.ui { ... }`. Elements
are boxes laid out with a small flexbox; each can hold text, children, and a
click action.

```lua
wlens.ui {
  { id = "bar", anchor = "bottom", width = "100%", height = 26, padding = { 0, 10 },
    background = "#181825", reserve = true,
    { text = "{name}", grow = true, text_align = "left", on_click = "copy path" },
    { text = function(img) return img and img.width > 1000 and "large" or "" end },
  },
}
```

`wlens.ui` **replaces the whole UI**, including the theme's menu, help and
settings (so `m`, `?` and `,` have nothing to open). To add something to the
theme's UI instead, put it in a [slot](#regions-and-slots).

To build your own layout from scratch, start from the `minimal` preset
(everything reset, colours from a palette if you give one) and add your UI:

```lua
local w = wlens.widgets
wlens.use("minimal", { palette = "tokyo-night" })
wlens.ui(w.list(
  w.layout {
    top = w.bar { left = { w.menu_button() }, center = { w.filename() }, right = { w.help_button() } },
    view = w.list(w.nav_zones(), w.menu(), w.help(), w.toast()),
  }
))
```

Complete custom layouts built this way (a filmstrip editor, a gallery-wall
caption) are in [`examples/layouts/`](../examples/layouts).

## Regions and slots

The window is laid out like any element: a column of top / middle / bottom,
where the middle row is left / **the main view** / right. The main view is an
element with `view = "main"`; the image or the thumbnail grid is drawn inside
its rectangle (and clipped to it), so bars and sidebars are simply its
siblings. `w.layout` builds that frame:

```lua
local w = wlens.widgets
wlens.ui(w.list(
  w.layout {
    top    = my_toolbar,                         -- an element or a list
    bottom = { w.scrubber(), w.bar { … } },
    left   = w.slot("left", { dir = "column", background = w.style.bar_background }),
    right  = w.slot("right", { dir = "column", background = w.style.bar_background }),
  },
  w.menu(), w.help(), w.toast()                  -- floating things stay top-level
))
```

`w.layout{ view = … }` takes elements floating over the main view itself:
the presets put the edge click zones, menus, the lightbox controls and toasts
there, so they centre on the image area and never cover a sidebar. Each
floats at its own `anchor` (the top-left corner if it has none).

Each region is an element with the id `panels`, so `ui toggle panels` (the
`b` key) hides the bars and sidebars together and the main view takes the
whole window.

Without a `view = "main"` element the main view is the whole window (minus
anything that `reserve`s space), as in the `minimal` preset.

**Slots** are named places a theme leaves open. Scripts fill them without
touching the theme, and what they add survives theme changes (`--theme`,
`wlens.use`):

```lua
wlens.slot("right", {                  -- a details sidebar, styled by the theme
  dir = "column", width = 220, gap = 8, align = "start",
  w.label("{name}", { max_width = 200 }),
  w.label("{width}×{height}"),
})
wlens.slot("bar", w.button("⇩", "lua download()", "Download"))
wlens.clear_slot("right")
```

`w.slot(name, style, outer)` declares one: hidden while empty, `style` is the
container (the theme decides what a sidebar looks like), `outer` an optional
wrapper such as `{ padding = 18 }` for a floating panel. The `simple`
and `lightbox` presets have `top` (above the main view, e.g. a page header),
`left`, `right`, `overlay` (a layer over the main view for scripts' menus,
dialogs and floating labels) and `bar` (simple) / `caption` (lightbox).
`minimal` has none: it's where you build your own.

## Sidebar kit

For script sidebars, use the kit instead of styling elements by hand: the
look (spacing, rounding, highlight colours) comes from `w.style`, so every
script's sidebar follows the theme and palette, and collapsing works the
same everywhere.

```lua
local w = wlens.widgets
wlens.slot("left", w.sidebar {
  id = "mine", title = "My source", icon = "https://example.org/favicon.ico",
  when = w.browsing,                                        -- optional: only in the grid
  sections = {
    w.search { on_submit = search },
    w.nav_list { items = function(img) return {
      { label = "Recent", icon = w.icons.calendar, active = mode == "recent", on_click = … },
    } end },
    w.divider(),
    w.section("Download"),
    w.text(function() return done .. " / " .. total end),
    w.progress(function() return done / total end),
    w.action("Download all", download, { primary = true, wide = true }),
  },
  rail = { w.nav_list { items = …, compact = true } },     -- shown when collapsed
})
wlens.bind("Tab", function() w.toggle_sidebar("mine") end)
```

Each section may have its own `show` (e.g. one panel at the top level,
another inside a level).

## Element keys

**Placement** (top-level elements): `anchor` (`top-left`, `top`, `top-right`,
`left`, `center`, `right`, `bottom-left`, `bottom`, `bottom-right`), `x`, `y`
(offset inwards), `reserve = true` (the image fits around it, like a bar) or `reserve = "gallery"` (only the thumbnail grid keeps clear of it; images still float under it).

**Size and layout:** `width`, `height` (pixels, `"50%"`, or `"auto"`),
`padding` (`n`, `{v, h}`, or `{t, r, b, l}`), `gap`, `direction` or `dir`
(`row` / `column`), `align` (cross axis: `start` `center` `end`), `justify` (main
axis), `grow = true` (take the remaining space). Array entries are children;
a bare string is a text child.

**Style:** `background` (or `bg`), `color` (text), `opacity`, `radius`,
`border = { width, color }` (or a number), `shadow = { size, color, x, y }`,
`hover = { …style keys… }` (animated on hover).

Use `selected = { when = function(img) return … end, …style keys… }` for a
control whose appearance follows a value. Its hit area stays in place.
A selected `justify` moves the control's children along the main axis, as
in the settings switches. The element's `anim` controls the transition.

**Content:** `text` — a template with `{name}`, `{path}`, `{dir}`, `{index}`,
`{count}`, `{width}`, `{height}`, `{zoom}`, `{size}`, `{fit}`, `{filter}`,
`{mode}`, `{level}`, `{depth}`, `{fitted}`, `{meta.KEY}` (a field of the
item's `meta`), and for videos `{position}`, `{duration}`, `{progress}` (0–1)
and `{volume}` (0–100) — or a function `function(img) return "…" end`.
`text_align`, `font`, `font_size`.

**Live values:** any key that takes a function (`text`, `show`, `image`,
`children`, a slider's `value`…) is re-evaluated whenever something might
have changed it: the current item, the window size, or *any* script code
running (a keybinding, a click, a timer, a fetch or download callback, a
hook). So a function can read plain Lua variables and the UI follows them,
with nothing to refresh by hand:

```lua
local done, total = 0, 0
wlens.slot("overlay", { anchor = "top-right", x = 12, y = 12,
  show = function() return total > 0 end,
  text = function() return done .. " / " .. total end })
-- … later, in a download callback:
done = done + 1   -- the label updates by itself
```

Nothing re-runs a function on its own, though: a clock needs a timer to tick
that re-arms itself; any code running refreshes the UI:

```lua
local function tick() wlens.after(1, tick) end
tick()
```

**Behaviour:**

| Key | |
|---|---|
| `on_click` | A command string or Lua function. Clicks on a picture or label inside it count as clicks on it (like a web button); an element with a `background` and no `on_click` is a panel and stops clicks from reaching what's behind it |
| `anchor` on a child | The child floats over its parent's box (placed like a top-level element over the window) instead of joining the row/column; it may `reserve` space |
| `view` | `"main"`: this element is where the image / grid is drawn |
| `image`, `fit` | A picture (path, URL, `{key}` template or function) filling the element, `cover` (default) or `contain`. Loaded through the thumbnail cache, with `http_headers` applied. Give the element a `width`/`height` |
| `input` | A text field: `{ placeholder, on_submit, on_change }` (functions of the text, or command templates with `{value}`). Needs an `id`; the text is `wlens.input(id)`. Click or `ui focus ID` to type; Enter submits, Escape leaves, Ctrl+U clears, Ctrl+Backspace deletes a word |
| `attach`, `attach_align` | `"element id"`: placed against that element — above it if there's room, else below — like a dropdown on its button (`w.menu` attaches to its `menu_button`). Centred over it by default; `attach_align = "start"` / `"end"` lines it up with an edge. Always kept inside the window |
| `wrap` | A row whose children flow onto new lines when they don't fit (give the row a px `width`) — e.g. tag chips |
| `responsive` | Fit a root or floating panel to its available width. Its contents are measured at that width so wrapping rows can grow vertically. |
| `stack` | Overlay children in one box sized to the largest child, including hidden pages. Use `show` to select a page. |
| `scroll` | Content taller than the box scrolls with the mouse wheel and is clipped to it (the sidebar kit's body uses it) |
| `max_height` | Cap on the height (px or `%`): an auto-height panel grows with its content but stops at the space available |
| `layer` | `"top"`: drawn after the rest of the UI, text included, so it covers it. Menus, help, toasts and tooltips use it |
| `children` | A function(img) returning a list of elements, re-evaluated like any live value (e.g. chips from `img.meta`, or as many thumbnails as fit). Rebuilt elements keep their hover, fade and scroll state |
| `shrink` | Text that gives up width when its row is too full, shortening with "…" (text with a `max_width` does this anyway). Rows never let their contents overlap: shrinkable text gives way first |
| `priority` | When a row is still too full after shortening text, items with a priority hide, lowest first (the presets give file size and dimensions 1, zoom 2) |
| `max_width`, `truncate` | Longest the text may get (logical px); beyond it it's shortened with "…" in the `middle` (default, keeps extensions) or at the `end` |
| `ring` | A circular progress ring in the element's box: `{ value = fn \| "{key}", fill, track, thickness }`; text/children sit in the middle. `spin = true` makes it a rotating loading arc (`w.spinner`) |
| `weight` | Font weight: `light` `normal` `medium` `semibold` `bold` or 100–900 |
| `slider` | A bar showing a 0–1 value, set by clicking or dragging: `{ value = "{progress}", on_change = "video seek {pct}%", track = colour, fill = colour }`. `value` may be a function of the info; `on_change` a command template (`{pct}` 0–100, `{value}` 0–1) or a function(value). Padding enlarges the clickable area around the track |
| `tooltip` | Text (templates work) shown after hovering |
| `show` | `"always"`, `"never"`, `"hover"` (fades in while the mouse moves), a bool, or `function(img) … end`. With an `id`, `ui hide ID` / `ui toggle ID` also hide it; on an element with a `show` function they work on top of it (hidden if either says so) |
| `id` | Lets `ui show/hide/toggle ID` and `wlens.visible(ID)` refer to it |
| `cursor` | `"pointer"` / `"default"`, or a function returning one |
| `follow_hover` | Light up (hover style) when the parent is hovered |
| `drag_through` | `true` lets a drag that starts on this element move the underlying view. A click still runs its action. |
| `pass_through` | `true`: the mouse ignores this element (a tint, a caption over the image); clicks, scrolling and dragging reach whatever is below. Its children can still be clicked |
| `anim` | Seconds for this element's fades, overriding `anim.ui` |

Hidden elements take no space in their parent. The topmost element with a
background or click action receives a click, so panels don't leak clicks to
what's behind them.

## The widget library

`wlens.widgets` builds common pieces. Every widget returns a plain element
table, so `wlens.extend(widget, { ... })` restyles it.

| Widget | |
|---|---|
| `layout { top, bottom, left, right, view }` | The window's regions around the main view, and what floats over it |
| `slot(name, style, outer)` + `wlens.slot(name, widget)`, `wlens.clear_slot(name)` | Named places scripts can fill |
| `bar { left, center, right, position, height, … }` | Three-section bar (centre is truly centred) |
| `filename()` (hidden for folder/gallery tiles), `counter()`, `zoom()`, `dims()`, `size()`, `level()` | Labels (dims/size hide when unknown; zoom only in image view) |
| `play_button()`, `timecode()`, `scrubber { width, height, thickness }`, `volume { width }` | Video: play / pause, position / length (click to pause), seek bar, speaker (click to mute) and volume bar — shown only for videos |
| `search { id, placeholder, on_submit, on_change, width }` | A text field (see `input`) |
| `sidebar`, `nav_list`, `section`, `divider`, `text`, `action`, `progress`, `toggle_sidebar(id)` | The sidebar kit (above) |
| `spinner { size, color }`, `loading()` | A rotating loading arc; a "Loading…" pill over the view while the current image is on its way (in both presets) |
| `ring_button { id, value, icon, on_click }` | A round button with a progress ring (e.g. downloads in a caption) |
| `popover { id, attach, content, width }` | A panel of anything opened against an element (`ui toggle ID`), closed by clicking elsewhere |
| `details { image, fit, facts, lists, on_chip, width }` | The current item's `meta`: picture, facts (`date  2025-06-01`), and lists (people, tags…) as clickable chips with optional avatars |
| `settings { tabs }` | The settings dialog (`ui toggle settings`, `,`). scripts add a tab with `w.add_settings_tab("Title", function() return { rows } end)` |
| `setting(label, control, hint)`, `toggle(key, {on, off})`, `choice(key, { {label, value}… })`, `range(key, min, max, {step})` | Settings rows and controls bound to an option: they show its current value and run `set key value` (applied at once, remembered next run) |
| `label(text, opts)` | Any text |
| `button(icon_or_text, action, tooltip, opts)` | Tooltip includes the bound key. Also takes one table: `w.button{ icon = …, action = …, tooltip = …, color = … }` (likewise `rail_button{ icon, … }` and `action{ label, … }`) |
| `navigation_button()` | Back returns to the thumbnail grid, then goes up one folder. The toolbar reserves its space at the root. |
| `menu_button()`, `back_button()`, `refresh_button()`, `gallery_button()`, `help_button()` | Individual buttons for custom layouts (each has that id) |
| `rail_button(icon, action, tooltip)` | A button for a collapsed sidebar's rail |
| `icon(glyph_or_url, size, color)` | An icon: a glyph, or a picture from a path or URL |
| `menu { items, position, margin, width }` | Items `{ label, action [, hint] }` or `"-"` (default `w.default_menu`); key hints come from your bindings |
| `help { columns, column_width }` | Live keybinding sheet |
| `toast()` + `wlens.toast(msg, secs)` | Transient messages |
| `nav_zones { width, indicator, reveal, … }` | Click the edges to go back/forward |
| `browse(path)` | Folder levels (see [Galleries](galleries.md)) |
| `slideshow(secs)` | Toggle a slideshow |
| `list(…)` | Build a UI list, skipping `false`/`nil` entries |
| `viewing`, `browsing`, `video_mode` | Ready-made `show` functions: only while viewing an image / video, only in the grid, or only for a video outside the grid (`show = w.viewing`) |
| `palette(spec)`, `apply_palette(spec)`, `try_palette(spec)`, `palette_names()` | Resolve a palette to its colours; apply one to `w.style` and the theme (`try_palette` only warns if it's missing); list the palettes (as `--list-palettes`) |
| `reset()` | Back to the default `w.style`, no overlays (presets call it first) |
| `dismiss()` | Close the topmost menu/overlay; returns whether one was open |

`w.icons.*` are Nerd Font glyphs (`menu`, `grid`, `back`, `refresh`,
`rotate_right`, `folder`, `close`, …); replace them with text if you don't
have a Nerd Font. `w.style` holds the shared look the widgets use — colours (`fg`,
`dim`, `accent`, `hover`, `panel`, `bar_background`, …) and corner rounding
(`radius` for panels, `button_radius` for buttons and menu rows,
`pill_radius` for pills and toasts; `999` means fully round) — which
`w.apply_palette` fills from a palette's `--radius`. `w.key_for(action)` is the
key bound to an action (for your own hints).

### Edge zones

```lua
w.nav_zones {
  width = "15%",          -- how much of each edge is clickable
  indicator = "none",     -- "none" (default, invisible) | "chevron" | "tint"
  reveal = "hover",       -- chevron: "hover" | "mouse" | "always"
  pointer = true,         -- hand cursor where clicking goes somewhere
}
```

Click the left or right edge to navigate at any image zoom level. Dragging
from either edge pans the image. The gallery uses its own navigation. At the
start or end of the list, clicking an edge with nowhere to go reports that boundary.
