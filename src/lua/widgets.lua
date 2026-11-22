-- wlens.widgets: building blocks for bars, menus and overlays.
--
-- Every widget is a function returning a plain `wlens.ui` element table, so
-- you can pass the result through wlens.extend() to restyle it, or skip the
-- widget and write the table yourself. Nothing here is special to Rust.
--
--   local w = wlens.widgets
--   wlens.ui {
--     w.nav_zones { width = "18%" },
--     w.bar {
--       left   = { w.menu_button(), w.filename() },
--       right  = { w.dims(), w.zoom(), w.counter(),
--                  w.button(w.icons.rotate_right, "rotate 90", "Rotate") },
--     },
--     w.menu(), w.help(), w.toast(),
--   }

local W = {}
local E = wlens.extend

-- Material Design glyphs from Nerd Fonts. Swap for plain text if you don't
-- have a Nerd Font: wlens.widgets.icons.menu = "≡"
W.icons = {
  menu = "\u{f035c}", prev = "\u{f0141}", next = "\u{f0142}", close = "\u{f0156}",
  rotate_right = "\u{f0467}", rotate_left = "\u{f0465}", flip = "\u{f10e7}",
  zoom_in = "\u{f06ed}", zoom_out = "\u{f06ec}", fit = "\u{f18f5}", actual = "\u{f03a6}",
  copy = "\u{f018f}", keyboard = "\u{f097b}", fullscreen = "\u{f0293}",
  play = "\u{f040a}", pause = "\u{f03e4}", info = "\u{f02fd}",
  volume = "\u{f057e}", muted = "\u{f0581}",
  checker = "\u{f013a}", wallpaper = "\u{f0e09}", help = "\u{f0625}", more = "\u{f01d9}",
  image = "\u{f0976}", grid = "\u{f11d9}", wrap = "\u{f0456}",
  back = "\u{f004d}", folder = "\u{f024b}", album = "\u{f02ea}",
  refresh = "\u{f0450}", calendar = "\u{f00f6}",
}

-- Shared look. Presets overwrite these before building their UI.
W.style = {
  icon_font = nil,                 -- nil = theme font + automatic fallback
  icon_size = 17,
  bg = "#121214",                  -- window background
  bar_background = "#1b1b1f",
  bar_height = 28,
  fg = "#d6d6db",
  dim = "#8b8b94",
  accent = "#8aadf4",
  hover = "#ffffff1a",
  panel = "#18181cf2",             -- menus, help, toasts
  panel_border = "#ffffff14",
  scrim = "#00000073",
  cell = "#ffffff0d",              -- gallery tile background
  checker = { "#262629", "#323236" },
  -- Corner rounding, shared by every widget (palettes set all three):
  radius = 8,                      -- panels: menus, help
  button_radius = 6,               -- buttons, menu rows, the file name
  pill_radius = 999,               -- pills: lightbox controls, toasts (999 = fully round)
  font = nil,                      -- a palette's --font-sans, if any
  palette = nil,                   -- set by apply_palette
}

---------------------------------------------------------------- palettes

local function config_dir()
  return (os.getenv("XDG_CONFIG_HOME") or ((os.getenv("HOME") or "") .. "/.config")) .. "/wlens"
end

-- Load a palette description: { name, base?, dark = {…}, light = {…} }.
local function load_palette_file(path)
  path = path:gsub("^~", os.getenv("HOME") or "~")
  if path:match("%.json$") then
    local f = io.open(path, "r")
    if not f then error("palette file not found: " .. path, 3) end
    local data = wlens.json.decode(f:read("a"))
    f:close()
    -- { name, tokens = { dark, light } } or { name, dark, light }
    return { name = data.name, dark = (data.tokens or data).dark, light = (data.tokens or data).light }
  end
  local ok, pal = pcall(dofile, path)
  if not ok then error("palette file " .. path .. ": " .. tostring(pal), 3) end
  return pal
end

-- Resolve a palette to its colour table.
--   "gruvbox"             ~/.config/wlens/themes/gruvbox.lua (a palette theme)
--   "gruvbox:light"       a specific mode (default: dark if present)
--   "/path/file.lua|json" a palette file ("…:light" works too)
--   { background = …, primary = …, … }   colours directly (or a whole palette
--                         table with dark/light)
-- Colour keys may be plain (`muted_foreground`) or CSS-style (`--muted-foreground`).
function W.palette(spec)
  local entry
  local name, mode
  if type(spec) == "table" then
    entry = spec
  else
    name, mode = tostring(spec):match("^(.-):(%a+)$")
    if not (mode == "dark" or mode == "light") then name, mode = tostring(spec), nil end
    if name:match("%.lua$") or name:match("%.json$") then
      entry = load_palette_file(name)
    else
      local ok, mod = pcall(require, "themes." .. name)
      if not ok or type(mod) ~= "table" then ok, mod = pcall(require, "wlens.palettes." .. name) end
      if not ok or type(mod) ~= "table" then
        error("unknown palette '" .. name .. "' (built-in: " .. table.concat(W.builtin_palettes, ", ")
          .. "; or put a palette theme in " .. config_dir() .. "/themes/" .. name .. ".lua, or pass a file path)", 2)
      end
      entry = mod
    end
  end
  if not (entry.dark or entry.light) then return entry end -- plain colour table
  local toks = entry[mode or "dark"] or (not mode and entry.light)
  if not toks then error("palette '" .. tostring(name) .. "' has no " .. tostring(mode) .. " mode", 2) end
  return toks
end

W.builtin_palettes = { "gruvbox", "strawberry-milk", "tokyo-night" }

-- Palette names: the built-in ones, then palette themes in
-- ~/.config/wlens/themes (files that return a colour table).
function W.palette_names()
  local names, seen = {}, {}
  for _, id in ipairs(W.builtin_palettes) do names[#names + 1] = id; seen[id] = true end
  local ok, entries = pcall(wlens.list_dir, config_dir() .. "/themes")
  if not ok then return names end
  for _, e in ipairs(entries) do
    local id = e.name:match("^(.*)%.lua$")
    if id then
      local okp, mod = pcall(require, "themes." .. id)
      if okp and type(mod) == "table" and not seen[id] then names[#names + 1] = id end
    end
  end
  return names
end

local function rem_px(v, default)
  if type(v) ~= "string" then return default end
  local n, unit = v:match("^([%d%.]+)(%a*)$")
  n = tonumber(n)
  if not n then return default end
  return unit == "rem" and n * 16 or n
end

local function first_font(css)
  if type(css) ~= "string" then return nil end
  local f = css:match("^%s*['\"]?([^,'\"]+)")
  if f == "system-ui" or f == "sans-serif" then return nil end
  return f
end

-- Map a palette onto W.style and the theme. Presets call this before
-- building their UI so every widget picks the colours up.
function W.apply_palette(spec)
  local t = W.palette(spec)
  local c = wlens.color
  -- Accept `--muted-foreground`, `muted-foreground` and `muted_foreground`.
  local function raw(name)
    local bare = name:gsub("^%-%-", "")
    return t[name] or t[bare] or t[(bare:gsub("%-", "_"))]
  end
  local function tok(name, fallback)
    local v = raw(name)
    return v and c(v) or fallback
  end
  local bg = tok("--background", W.style.bg)
  local fg = tok("--foreground", W.style.fg)
  local card = tok("--card", bg)
  local popover = tok("--popover", card)
  local muted = tok("--muted", tok("--secondary", card))
  local accent = tok("--primary", W.style.accent)
  local hover = tok("--accent", W.style.hover)
  local border = tok("--border", W.style.panel_border)
  local radius = rem_px(raw("--radius"), 6)
  W.style.palette = t
  W.style.bg = bg
  W.style.fg = fg
  W.style.dim = tok("--muted-foreground", W.style.dim)
  W.style.accent = accent
  W.style.hover = hover
  W.style.bar_background = tok("--sidebar", card)
  W.style.panel = c(popover, 0.96)
  W.style.panel_border = border
  W.style.cell = card
  W.style.checker = { muted, card }
  W.style.radius = math.max(2, radius * 1.5)
  W.style.button_radius = math.max(2, radius)
  W.style.pill_radius = math.max(3, radius * 2)
  wlens.theme {
    window = { background = bg },
    font = { color = fg },
    image = { checker = W.style.checker },
    tooltip = { background = c(popover, 0.97), color = tok("--popover-foreground", fg), radius = math.max(3, radius) },
    gallery = {
      background = card, selected = tok("--ring", accent), hover = border,
      label_color = W.style.dim, radius = math.max(2, radius * 1.25),
    },
  }
  W.style.font = first_font(raw("--font-sans"))
  return t
end

-- Like apply_palette, but a missing/broken palette only warns: the theme
-- falls back to its own colours instead of stopping wlens from starting.
function W.try_palette(spec)
  local ok, res = pcall(W.apply_palette, spec)
  if ok then return res end
  local msg = tostring(res):gsub("^.-:%d+: ", "")
  print("wlens: " .. msg)
  W._palette_error = msg
  return nil
end

-- Overlays that Escape (W.dismiss) should close, most recent first.
W.overlays = {}

local default_style = E(W.style, {})
-- Presets call this first: default style, no registered overlays.
-- For `show =`: only while viewing an image / video, or only in the grid.
--   { …, show = w.viewing }
function W.viewing(img) return img ~= nil and img.mode ~= "gallery" end
function W.browsing(img) return img ~= nil and img.mode == "gallery" end

function W.reset()
  W.style = E(default_style, {})
  W.overlays = {}
  W.slot_nodes = {}
end

---------------------------------------------------------------- regions & slots
--
-- The window is a layout like any other: a column of top / middle / bottom,
-- where the middle row is left / the main view / right. The main view
-- (`view = "main"`) is where the image or the thumbnail grid is drawn.
--
--   wlens.ui(w.list(
--     w.layout { bottom = w.bar{…}, left = w.slot("left"), right = w.slot("right") },
--     w.menu(), w.help(), w.toast()        -- floating things stay top-level
--   ))
--
-- Slots are named places themes leave open; scripts fill them without
-- rebuilding the theme, and the widgets stay when the theme changes:
--
--   wlens.slot("right", my_panel)      -- e.g. a details sidebar
--   wlens.slot("caption", w.button(w.icons.refresh, "refresh", "Reload"))

W.slot_widgets = {} -- name -> widgets added by scripts (survives theme changes)
W.slot_nodes = {}   -- name -> containers in the current UI (reset per theme)

-- A container for slot `name`. Hidden while empty. `o` styles the container
-- (e.g. { dir = "column", width = 260, background = w.style.panel });
-- `outer` optionally wraps it (e.g. { padding = 16 } for a floating panel),
-- hidden along with it.
function W.slot(name, o, outer)
  local el = E({ gap = 8, align = "center" }, o or {})
  local added = W.slot_widgets[name] or {}
  for _, wdg in ipairs(added) do el[#el + 1] = wdg end
  local top = outer and E(outer, { el }) or el
  top.id = top.id or ("slot_" .. name) -- `ui toggle slot_right` hides a sidebar
  if #el == 0 then top.show = "never" end
  W.slot_nodes[name] = W.slot_nodes[name] or {}
  table.insert(W.slot_nodes[name], { el = el, top = top })
  return top
end

-- Add a widget to a slot (any theme that has it shows it).
function wlens.slot(name, widget)
  W.slot_widgets[name] = W.slot_widgets[name] or {}
  table.insert(W.slot_widgets[name], widget)
  for _, s in ipairs(W.slot_nodes[name] or {}) do
    s.el[#s.el + 1] = widget
    if s.top.show == "never" then s.top.show = nil end
  end
  if wlens._ui_spec then wlens.ui(wlens._ui_spec) end
end

-- Remove everything scripts added to a slot.
function wlens.clear_slot(name)
  W.slot_widgets[name] = nil
  for _, s in ipairs(W.slot_nodes[name] or {}) do
    for i = #s.el, 1, -1 do s.el[i] = nil end
    s.top.show = "never"
  end
  if wlens._ui_spec then wlens.ui(wlens._ui_spec) end
end

-- The window's regions around the main view. Each side takes an element or
-- a list of them (stacked away from the view); `nil` leaves it empty.
-- `view` takes elements placed over the main view itself.
function W.layout(o)
  o = o or {}
  local function list(v)
    if v == nil or v == false then return {} end
    -- A plain list has only numbered entries; an element has named keys.
    for k in pairs(v) do
      if type(k) ~= "number" then return { v } end
    end
    return v
  end
  -- A bar or panel placed in a region becomes part of the layout: it
  -- takes its row / column instead of floating over the window, so
  -- `top = { w.bar{ position = "top" } }` just works.
  local function docked(items)
    for _, it in ipairs(items) do
      if type(it) == "table" then it.anchor, it.reserve = nil, nil end
    end
    return items
  end
  -- Every region is a "panels" element, so `ui toggle panels` (the `b` key)
  -- hides bars and sidebars together and the main view takes the window.
  local function sidebar(items)
    local col = { id = "panels", dir = "row", height = "100%", align = "start" }
    for _, it in ipairs(docked(items)) do
      if it.height == nil then it.height = "100%" end
      col[#col + 1] = it
    end
    return col
  end
  local middle = { id = "middle", dir = "row", grow = true, width = "100%", align = "start" }
  middle[#middle + 1] = sidebar(list(o.left))
  -- `view` = elements floating over the main view only (edge click zones,
  -- menus, toasts), so they never cover the bars or sidebars. Each floats
  -- (its own `anchor`, else the top-left corner) rather than queueing up
  -- in a row beside the others.
  local view = { view = "main", grow = true, height = "100%" }
  for _, it in ipairs(list(o.view)) do
    if type(it) == "table" and it.anchor == nil then it.anchor = "top-left" end
    view[#view + 1] = it
  end
  middle[#middle + 1] = view
  middle[#middle + 1] = sidebar(list(o.right))
  local function band(items)
    local b = { id = "panels", dir = "column", width = "100%" }
    for _, it in ipairs(docked(items)) do b[#b + 1] = it end
    return b
  end
  return { id = "layout", dir = "column", width = "100%", height = "100%",
           band(list(o.top)), middle, band(list(o.bottom)) }
end

-- Build a root list for wlens.ui, skipping falsy entries:
--   wlens.ui(w.list(show_zones and w.nav_zones(), w.bar{...}))
function W.list(...)
  local out = {}
  for i = 1, select("#", ...) do
    local v = select(i, ...)
    if v then out[#out + 1] = v end
  end
  return out
end

local function run(action)
  if type(action) == "function" then action() else wlens.exec(action) end
end

-- The first key bound to exactly this command, for menu hints and tooltips.
function W.key_for(action)
  if type(action) ~= "string" then return nil end
  for _, b in ipairs(wlens.bindings()) do
    if b.command == action then return b.key end
  end
end

local function with_key(tip, action)
  local k = W.key_for(action)
  if tip and k then return tip .. "  (" .. k .. ")" end
  return tip
end

-- Buttons take their main parts in order or by name, whichever reads better:
--   w.button(w.icons.rotate_right, "rotate 90", "Rotate")
--   w.button{ icon = w.icons.rotate_right, action = "rotate 90", tooltip = "Rotate", color = "#f00" }
-- Returns (first, action, tooltip, options) either way.
local function named(first_key, a, action, tip, o)
  if type(a) ~= "table" then return a, action, tip, o or {} end
  local t = E(a, {})
  local first, act, tp = t[first_key], t.action or t.on_click, t.tooltip
  t[first_key], t.action, t.on_click, t.tooltip = nil, nil, nil, nil
  return first, act, tp, t
end

---------------------------------------------------------------- bar pieces

function W.label(text, o)
  return E({ text = text, color = W.style.fg }, o or {})
end

function W.filename(o)
  return E({
    text = "{name}", color = W.style.fg, padding = { 3, 8 }, radius = W.style.button_radius,
    max_width = 360, -- long names shorten in the middle, keeping the extension
    -- Describes what's open: hidden while a folder/gallery tile is merely selected.
    show = function(img) return img ~= nil and not img.container end,
    on_click = "copy path", tooltip = "{name} — click to copy the path",
    hover = { background = W.style.hover, color = W.style.fg },
  }, o or {})
end

function W.counter(o)
  -- Hidden while a list is empty (e.g. still loading).
  return W.label("{index}/{count}", E({ show = function(img) return img ~= nil and img.count > 0 end }, o or {}))
end
local function image_mode(img) return img ~= nil and (img.mode == "image" or img.mode == "video") end
function W.zoom(o)
  return W.label("{zoom}%", E({ color = W.style.accent, show = image_mode, priority = 2 }, o or {}))
end
local function video_mode(img) return img ~= nil and img.video and img.mode ~= "gallery" end
W.video_mode = video_mode

-- Seek bar: click or drag to scrub. `thickness` is the bar, `height` the
-- (taller) clickable area.
function W.scrubber(o)
  o = o or {}
  local h, t = o.height or 14, o.thickness or 4
  return E({
    id = "scrubber", height = h, width = o.width or "100%", padding = { (h - t) / 2, o.inset or 0 },
    show = video_mode, tooltip = "{position} / {duration}",
    slider = { value = "{progress}", on_change = "video seek {pct}%",
               track = o.track or wlens.color(W.style.fg, 0.2), fill = o.fill or W.style.accent },
  }, o.extra or {})
end

-- Volume: a speaker (click to mute) and a bar (click or drag, 0–100%).
--   w.volume { width = 64 }   -- the bar's width; `extra` restyles the row
function W.volume(o)
  o = o or {}
  local muted = function(img) return img ~= nil and (img.muted or img.volume == 0) end
  return E({
    dir = "row", gap = 2, show = video_mode,
    W.button(function(img) return muted(img) and W.icons.muted or W.icons.volume end,
             "video mute", "Mute", { font_size = W.style.icon_size - 2 }),
    { width = o.width or 64, height = 14, padding = { 5, 0 },
      tooltip = "Volume {volume}%",
      slider = { value = function(img) return img and (img.muted and 0 or img.volume) or 0 end,
                 on_change = "video volume {pct}",
                 track = wlens.color(W.style.fg, 0.2), fill = o.fill or W.style.fg } },
  }, o.extra or {})
end

-- Play / pause, showing what a click will do. Only shown for videos.
function W.play_button(o)
  return W.button(function(img) return img and img.paused and W.icons.play or W.icons.pause end,
                  "video toggle", "Play / pause", E({ id = "play_button", show = video_mode }, o or {}))
end

-- A text field. Its text is wlens.input(id). `on_submit(text)` runs on Enter,
-- `on_change(text)` on every edit (either may also be a command template
-- with {value}). Focus it with a click or the command `ui focus <id>`;
-- Escape leaves it.
--   w.search { id = "search", placeholder = "Search…", on_submit = function(q) … end }
function W.search(o)
  o = o or {}
  local st = W.style
  -- A field: magnifier + the input itself (which keeps the id for `ui focus`).
  return E({
    dir = "row", gap = 8, padding = { 7, 10 }, width = o.width or 220,
    radius = st.button_radius, background = wlens.color(st.fg, 0.06),
    border = { width = 1, color = wlens.color(st.fg, 0.10) },
    hover = { border = { width = 1, color = wlens.color(st.fg, 0.24) } },
    { text = "\u{f0349}", font = st.icon_font, font_size = 15, color = st.dim },
    { id = o.id or "search", grow = true, color = st.fg, tooltip = o.tooltip,
      input = { placeholder = o.placeholder or "Search…", on_submit = o.on_submit, on_change = o.on_change } },
  }, o.extra or {})
end

---------------------------------------------------------------- sidebar kit
--
-- Building blocks for script sidebars. Scripts say *what* is there; the look
-- (spacing, rounding, highlight colours) comes from w.style, so palettes and
-- themes restyle every script's sidebar the same way.
--
--   wlens.slot("left", w.sidebar {
--     id = "mine", title = "Library", icon = url_or_glyph,
--     when = function(img) return img.mode == "gallery" end,   -- optional
--     sections = { w.search{…}, w.nav_list{…}, w.divider(), w.details{…} },
--     rail = { w.nav_list{ …, compact = true } },               -- when collapsed
--   })
--   wlens.bind("Tab", function() w.toggle_sidebar("mine") end)

W.sidebars = {} -- id -> { collapsed = bool }

function W.toggle_sidebar(id)
  local sb = W.sidebars[id]
  if not sb then return end
  sb.collapsed = not sb.collapsed
  wlens.store.set("sidebar." .. id .. ".collapsed", sb.collapsed) -- remembered next run
end

-- An icon: a glyph, or a picture when given a URL / path (e.g. a favicon).
local function icon_el(icon, size, color)
  if type(icon) == "string" and (icon:match("^%a[%w+.-]*://") or icon:match("^[/~]")) then
    return { image = icon, fit = "contain", width = size, height = size, radius = math.floor(size / 5) }
  end
  return { text = icon, font = W.style.icon_font, font_size = size or W.style.icon_size,
           color = color or W.style.fg }
end
W.icon = icon_el

-- A square rail button (collapsed sidebars): every one the same size.
function W.rail_button(icon, action, tip, o)
  icon, action, tip, o = named("icon", icon, action, tip, o)
  local st = W.style
  local fg = o.active and st.accent or (o.color or st.fg)
  return E({
    width = 40, height = 40, radius = st.button_radius, justify = "center",
    background = o.active and wlens.color(st.accent, 0.16) or "transparent",
    hover = { background = o.active and wlens.color(st.accent, 0.22) or wlens.color(st.fg, 0.07) },
    on_click = action and function() run(action) end or nil, tooltip = tip,
    icon_el(icon, o.size or 18, fg),
  }, o.extra or {})
end

-- A sidebar with a header (icon, title, collapse chevron), `sections` (each
-- may have its own `show`), and a `rail` shown instead when collapsed.
function W.sidebar(o)
  local id = assert(o.id, "w.sidebar needs an id")
  W.sidebars[id] = W.sidebars[id] or { collapsed = wlens.store.get("sidebar." .. id .. ".collapsed") == true }
  local sb = W.sidebars[id]
  local st = W.style
  local width = o.width or 248
  local function toggle() W.toggle_sidebar(id) end
  local quiet = { color = st.dim, hover = { background = st.hover, color = st.fg } }
  local title = { dir = "column", gap = 1, grow = true, align = "start",
                  { text = o.title or "", color = st.fg, font_size = 16, weight = 600 } }
  if o.subtitle then
    title[#title + 1] = { text = o.subtitle, color = st.dim, font_size = 12,
                          max_width = width - 80, truncate = "end" }
  end
  -- The sections scroll (the header stays) when they don't fit.
  local body = { dir = "column", gap = 14, width = "100%", align = "start", grow = true, scroll = true }
  for _, sec in ipairs(o.sections or {}) do body[#body + 1] = sec end
  local full = {
    id = id .. "_full", dir = "column", gap = 14, width = width, align = "start", max_height = "100%",
    show = function() return not sb.collapsed end,
    { dir = "row", width = "100%", gap = 10, padding = { 0, 0, 2, 0 },
      o.icon and icon_el(o.icon, 22) or nil,
      title,
      W.button(W.icons.prev, toggle, "Collapse sidebar", quiet) },
    body,
  }
  -- Collapsed: the brand icon (click to expand), the expand chevron, then
  -- the script's rail items — all the same square size.
  local rail = {
    id = id .. "_rail", dir = "column", gap = 4, align = "center",
    show = function() return sb.collapsed end,
  }
  if o.icon then rail[#rail + 1] = W.rail_button(o.icon, toggle, "Expand sidebar", { size = 26 }) end
  rail[#rail + 1] = W.rail_button(W.icons.next, toggle, "Expand sidebar", { color = st.dim })
  if #(o.rail or {}) > 0 then rail[#rail + 1] = W.divider { width = 28, padding = { 4, 0 } } end
  for _, r in ipairs(o.rail or {}) do rail[#rail + 1] = r end
  return { id = id, dir = "column", full, rail, max_height = "100%",
           show = o.when and function(img) return o.when(img) end or nil }
end

-- A list of rows: icon, label, optional detail, active highlight. `items`
-- is a function (re-run when the item list changes) returning
-- { { label, detail?, icon?, active?, tooltip?, on_click }, … }.
-- `compact = true` shows icons only (or the label if there's no icon), for rails.
function W.nav_list(o)
  local st = W.style
  return {
    dir = "column", gap = 2, width = o.compact and "auto" or "100%", align = o.compact and "center" or "start",
    show = o.show,
    children = function(img)
      local rows = {}
      for _, it in ipairs(o.items(img) or {}) do
        local fg = it.active and st.accent or st.fg
        if o.compact and it.icon then
          rows[#rows + 1] = W.rail_button(it.icon, it.on_click, it.tooltip or it.label, { active = it.active })
          goto continue
        end
        local row = {
          dir = "row", gap = 12, padding = o.compact and { 8, 9 } or { 7, 10 }, radius = st.button_radius,
          width = o.compact and "auto" or "100%",
          background = it.active and wlens.color(st.accent, 0.16) or "transparent",
          hover = { background = it.active and wlens.color(st.accent, 0.22) or wlens.color(st.fg, 0.06) },
          on_click = it.on_click,
          tooltip = (o.compact and (it.tooltip or it.label)) or it.tooltip,
        }
        if it.icon then
          -- Fixed-width icon column so labels line up.
          row[#row + 1] = { width = 20, justify = "center", icon_el(it.icon, 18, fg) }
        end
        if not o.compact or not it.icon then
          row[#row + 1] = { text = it.label, color = fg, weight = it.active and 600 or nil,
                            max_width = o.label_width or 150, truncate = "end",
                            grow = not it.detail or o.compact, text_align = "left" }
        end
        if it.detail and not o.compact then
          row[#row + 1] = { text = it.detail, color = it.active and st.accent or st.dim, font_size = 12,
                            grow = true, text_align = o.detail_align or "right", max_width = 150, truncate = "end" }
        end
        rows[#rows + 1] = row
        ::continue::
      end
      return rows
    end,
  }
end

-- A round button with a progress ring around an icon (e.g. downloads in a
-- caption / bar). `value` is a function returning 0..1.
function W.ring_button(o)
  local st = W.style
  return E({
    id = o.id, width = o.size or 30, height = o.size or 30, padding = 2, radius = st.pill_radius,
    ring = { value = o.value, fill = o.fill or st.accent, track = o.track or wlens.color(st.fg, 0.16),
             thickness = o.thickness or 2.5 },
    text = o.icon or "\u{f01da}", font = st.icon_font, font_size = o.icon_size or 14, color = st.fg,
    hover = { background = st.hover }, on_click = o.on_click, tooltip = o.tooltip, show = o.show,
  }, o.extra or {})
end

-- A popover: a panel of anything, opened against another element (like the
-- menu on its button), closed by clicking elsewhere or Escape. Toggle it with
-- `ui toggle <id>`. `content` is a list of elements, or a function(img)
-- returning one (re-run when the item changes).
--   wlens.slot("overlay", w.popover { id = "dl", attach = "dl_button", content = {…} })
function W.popover(o)
  local st = W.style
  local panel = {
    dir = "column", gap = 10, padding = 14, width = o.width or 260, align = "start",
    background = st.panel, radius = st.radius + 2,
    border = { width = 1, color = st.panel_border },
    shadow = { size = 18, color = "#00000099", y = 6 },
    attach = o.attach,
  }
  if type(o.content) == "function" then
    panel.children = o.content
  else
    for _, el in ipairs(o.content or {}) do panel[#panel + 1] = el end
  end
  return W._overlay(o.id, panel, { scrim = "transparent" })
end

-- "Loading…" over the view while the current image / video is still on its
-- way (slow servers), so it doesn't look like nothing happened.
function W.loading(o)
  local st = W.style
  return E({
    id = "loading", anchor = "center", padding = { 8, 16 }, radius = st.pill_radius,
    background = st.panel, border = { width = 1, color = st.panel_border },
    dir = "row", gap = 10, layer = "top",
    show = function(img) return img ~= nil and img.loading end,
    W.spinner { size = 16, color = st.accent },
    { text = "Loading…", color = st.fg },
  }, o or {})
end

-- A rotating loading arc.
function W.spinner(o)
  o = o or {}
  local st = W.style
  return E({ width = o.size or 20, height = o.size or 20,
             ring = { spin = true, fill = o.color or st.accent,
                      track = wlens.color(o.color or st.fg, 0.15), thickness = o.thickness or 2.5 } },
           o.extra or {})
end

-- Small pieces with the shared look.
function W.section(title, o)
  return E({ text = title:upper(), color = W.style.dim, font_size = 11, weight = 600,
             padding = { 4, 2, 0, 2 } }, o or {})
end

function W.divider(o)
  return E({ height = 1, width = "100%", background = W.style.panel_border }, o or {})
end

-- Text, dim or not; `text` may be a function of the current item.
function W.text(text, o)
  o = o or {}
  local t = E({ text = text, color = o.dim and W.style.dim or W.style.fg,
                text_align = "left", max_width = o.max_width or 230, truncate = o.truncate or "end" }, o)
  t.dim = nil
  return t
end

-- A button with a label (text or function). `primary = true` for the main action.
function W.action(label, on_click, o)
  local tip
  label, on_click, tip, o = named("label", label, on_click, nil, o)
  if tip then o.tooltip = tip end
  local st = W.style
  local el
  if o.primary then
    -- Solid accent, background-coloured text.
    el = { background = st.accent, color = st.bg, weight = 600,
           hover = { background = wlens.color(st.accent, 0.85) } }
  else
    el = { background = wlens.color(st.fg, 0.07), color = st.fg,
           hover = { background = wlens.color(st.fg, 0.14) } }
  end
  el = E({ text = label, on_click = on_click, padding = { 8, 14 }, radius = st.button_radius,
           width = o.wide and "100%" or nil }, el, o)
  el.primary, el.wide = nil, nil
  return el
end

-- A progress bar; `value` is a function returning 0..1.
function W.progress(value, o)
  local st = W.style
  return E({ width = "100%", height = 10, padding = { 3, 0 },
             slider = { value = value, track = wlens.color(st.fg, 0.12), fill = st.accent } }, o or {})
end

-- A details panel for the current item's `meta` (set by scripts on items):
--   w.details {
--     image = "image",                         -- meta key with a picture URL (optional)
--     fit = "contain",                         -- or "cover" to crop it to the box
--     facts = { "date", "rating" },           -- shown as "date  2025-06-01"
--     lists = { "people", "tags" },           -- entries: "name" or { name, image, … }
--     on_chip = function(list, entry, img) … end,   -- click a person / tag
--   }
-- Rebuilds itself whenever the current item changes; hidden without meta.
function W.details(o)
  o = o or {}
  local st = W.style
  local width = o.width or 248
  local function name_of(e) return type(e) == "table" and (e.name or e.title or tostring(e.id)) or tostring(e) end
  local function chip(list, e, img)
    local c = {
      dir = "row", gap = 6, padding = { 4, 10, 4, 4 }, radius = st.pill_radius,
      background = wlens.color(st.fg, 0.07),
      hover = { background = wlens.color(st.fg, 0.15) },
      on_click = o.on_chip and function() o.on_chip(list, e, img) end or nil,
    }
    if type(e) == "table" and e.image then
      c[#c + 1] = { image = e.image, width = 18, height = 18, radius = 9 }
    else
      c.padding = { 4, 10 }
    end
    c[#c + 1] = { text = name_of(e), color = st.fg, font_size = 12, max_width = width - 50, truncate = "end" }
    return c
  end
  return E({
    id = o.id or "details", dir = "column", gap = 10, align = "start", width = width,
    show = function(img) return img ~= nil and img.meta ~= nil end,
    children = function(img)
      local m = img and img.meta
      if not m then return {} end
      local out = {}
      if o.image and m[o.image] then
        out[#out + 1] = { image = m[o.image], fit = o.fit or "contain", background = wlens.color(st.fg, 0.04),
                          width = width, height = math.floor(width * 0.7), radius = st.radius + 2 }
      end
      out[#out + 1] = { text = m.title or img.name, color = st.fg, font_size = 15, weight = 600,
                        max_width = width, truncate = "end" }
      local facts = { dir = "column", gap = 4, width = "100%", align = "start" }
      for _, k in ipairs(o.facts or {}) do
        if m[k] ~= nil and m[k] ~= "" then
          facts[#facts + 1] = { dir = "row", gap = 8, width = "100%",
            { text = k:sub(1, 1):upper() .. k:sub(2), color = st.dim, width = 80, text_align = "left" },
            { text = tostring(m[k]), color = st.fg, grow = true, text_align = "left", max_width = width - 90, truncate = "end" } }
        end
      end
      if #facts > 0 then out[#out + 1] = facts end
      for _, k in ipairs(o.lists or {}) do
        local list = m[k]
        if type(list) == "table" and #list > 0 then
          out[#out + 1] = W.section(k)
          -- Chips flow in lines instead of one long column.
          local flow = { dir = "row", wrap = true, width = width, gap = 6, align = "start" }
          for _, e in ipairs(list) do flow[#flow + 1] = chip(k, e, img) end
          out[#out + 1] = flow
        end
      end
      return out
    end,
  }, o.extra or {})
end

-- Video position / length; click to play or pause. Only shown for videos.
function W.timecode(o)
  return W.label("{position} / {duration}", E({ color = W.style.accent,
    show = video_mode, on_click = "video toggle", tooltip = "Play / pause (space)" }, o or {}))
end
-- Dimensions/size hide when unknown (folders, not-yet-loaded items).
function W.dims(o)
  return W.label("{width}×{height}", E({ color = W.style.dim, priority = 1,
    show = function(img) return img ~= nil and img.width > 0 end }, o or {}))
end
function W.size(o)
  return W.label("{size}", E({ color = W.style.dim, priority = 1,
    show = function(img) return img ~= nil and img.size > 0 end }, o or {}))
end

-- An icon (or word) button: w.button(w.icons.rotate_right, "rotate 90", "Rotate")
-- or w.button{ icon = …, action = …, tooltip = …, …any element keys }.
function W.button(icon, action, tip, o)
  icon, action, tip, o = named("icon", icon, action, tip, o)
  return E({
    text = icon, on_click = function() run(action) end,
    tooltip = with_key(tip, action),
    font = W.style.icon_font, font_size = W.style.icon_size,
    padding = { 2, 7 }, radius = W.style.button_radius, color = W.style.fg,
    hover = { background = W.style.hover, color = W.style.fg },
  }, o)
end

-- Shown only inside a level (a folder, an album…): go back up.
function W.back_button(o)
  return W.button(W.icons.back, "back", "Back", E({
    id = "back_button",
    show = function(img) return img ~= nil and img.depth > 0 end,
  }, o or {}))
end

-- The current level's title ("Holiday 2026"), hidden at the top level.
function W.level(o)
  return W.label("{level}", E({
    color = W.style.dim, max_width = 220,
    -- From an image: click to see this level's grid (it never leaves the level).
    on_click = function()
      local c = wlens.current()
      if c and c.mode ~= "gallery" then wlens.gallery("open") end
    end,
    cursor = function(img) return (img and img.mode ~= "gallery") and "pointer" or "default" end,
    tooltip = "Show all", radius = W.style.button_radius, padding = { 3, 8 },
    hover = { background = W.style.hover, color = W.style.fg },
    show = function(img) return img ~= nil and img.level ~= nil end,
  }, o or {}))
end

function W.refresh_button(o)
  return W.button(W.icons.refresh, "refresh", "Refresh", E({ id = "refresh_button" }, o or {}))
end

-- Show the thumbnail grid (from an image). In the grid it does nothing:
-- opening an image is a click or Return, never this button.
function W.gallery_button(o)
  return W.button(W.icons.grid, "gallery open", "Show all", E({
    id = "gallery_button",
    cursor = function(img) return (img and img.mode ~= "gallery") and "pointer" or "default" end,
  }, o or {}))
end

-- Open the keybindings sheet.
function W.help_button(o)
  return W.button(W.icons.help, "ui toggle help", "Keybindings", E({ id = "help_button" }, o or {}))
end

function W.menu_button(o)
  o = o or {}
  local id = o.menu or "menu"
  o.menu = nil
  -- The menu opens against this button (see `attach` in W.menu).
  o.id = o.id or (id .. "_button")
  return W.button(W.icons.menu, "ui toggle " .. id, "Menu", o)
end

-- A bar with left / center / right sections. The centre is truly centred.
function W.bar(o)
  -- Sections keep their natural width with flexible space between them, so
  -- the bar is one row: when it's too full, long text shortens and items
  -- with a `priority` hide (lowest first) — nothing overlaps.
  local section = function(items, justify)
    local t = { justify = justify, gap = o.gap or 14 }
    for _, it in ipairs(items or {}) do t[#t + 1] = it end
    return t
  end
  local el = {
    id = o.id or "bar",
    -- Anchored as a top-level bar; `w.layout` drops the anchor so it takes
    -- its place in the column instead of floating.
    anchor = o.reserve ~= false and (o.position or "bottom") or nil,
    width = "100%", height = o.height or W.style.bar_height,
    padding = o.padding or { 0, 8 }, gap = 14,
    background = o.background or W.style.bar_background,
    reserve = o.reserve ~= false,
    show = o.show,
    section(o.left, "start"),
    { grow = true },
    section(o.center, "center"),
    { grow = true },
    section(o.right, "end"),
  }
  return el
end

---------------------------------------------------------------- overlays

-- A full-window layer that closes itself when you click outside its panel.
local function overlay(id, panel, o)
  W.overlays[#W.overlays + 1] = id
  -- The panel floats over the overlay (rather than being laid out in it), so
  -- in a small window it's kept on screen instead of running off an edge.
  local vert = { start = "top", center = "center", ["end"] = "bottom" }
  local horiz = { start = "left", center = "center", ["end"] = "right" }
  panel.anchor = vert[o.vertical or "center"] .. "-" .. horiz[o.horizontal or "center"]
  return {
    id = id, show = "never", layer = "top",
    width = "100%", height = "100%",
    background = o.scrim or "transparent",
    on_click = "ui hide " .. id,
    direction = "column",
    justify = o.vertical or "center", align = o.horizontal or "center",
    padding = o.margin or 0,
    panel,
  }
end
W._overlay = overlay -- for widgets defined above it (popover)

-- Close the topmost open overlay. Returns true if one was open.
function W.dismiss()
  for i = #W.overlays, 1, -1 do
    local id = W.overlays[i]
    if wlens.visible(id) then
      wlens.hide(id)
      return true
    end
  end
  return false
end

W.default_menu = {
  { "Copy path", "copy path" },
  { "Copy image", "copy image" },
  "-",
  { "Rotate right", "rotate 90" },
  { "Rotate left", "rotate -90" },
  { "Flip horizontally", "flip h" },
  { "Fit to window", "fit" },
  { "Actual size", "zoom actual" },
  { "Fullscreen", "fullscreen" },
  "-",
  { "Show all (grid)", "gallery open" },
  { "Back", "back" },
  { "Refresh", "refresh" },
  { "Slideshow", function() W.slideshow() end },
  "-",
  { "Settings…", "ui toggle settings" },
  { "Keybindings", "ui show help" },
  { "Quit", "quit" },
}

-- A popup menu. `items` are { label, action [, key hint] } or "-" separators.
-- `position` places the panel: "bottom-left" (above a bottom bar), "top-right", "center"...
function W.menu(o)
  o = o or {}
  local id = o.id or "menu"
  local panel = {
    -- Opens against its button (above it if there's room) when it has one;
    -- otherwise at `position`.
    attach = o.attach == nil and (id .. "_button") or o.attach or nil,
    -- No gaps between rows: the highlight moves straight from one to the next.
    direction = "column", padding = 5, gap = 0, width = o.width or 240,
    background = W.style.panel, radius = W.style.radius,
    border = { width = 1, color = W.style.panel_border },
    shadow = { size = 18, color = "#00000099", y = 6 },
  }
  for _, it in ipairs(o.items or W.default_menu) do
    if it == "-" then
      panel[#panel + 1] = {
        width = "100%", padding = { 4, 6 },
        { height = 1, width = "100%", background = W.style.panel_border },
      }
    else
      local label, action, hint = it[1], it[2], it[3] or W.key_for(it[2])
      panel[#panel + 1] = {
        width = "100%", padding = { 6, 10 }, radius = W.style.button_radius, gap = 12,
        hover = { background = W.style.hover },
        -- Like native menus: arrow cursor, near-instant highlight.
        cursor = o.cursor or "default",
        anim = o.anim or 0.06,
        on_click = function()
          wlens.hide(id)
          run(action)
        end,
        { text = label, grow = true, text_align = "left", color = W.style.fg },
        hint and { text = hint, color = W.style.dim, font_size = 12 } or nil,
      }
    end
  end
  local pos = o.position or "bottom-left"
  local v, h = pos:match("^(%a+)-(%a+)$")
  if not v then v, h = pos, "center" end
  local map = { top = "start", left = "start", center = "center", bottom = "end", right = "end" }
  local m = o.margin or { 8, 8, W.style.bar_height + 6, 8 }
  return overlay(id, panel, { vertical = map[v], horizontal = map[h], margin = m })
end

---------------------------------------------------------------- settings dialog
--
-- Controls bound to an option (wlens.opt[key]); changing one runs
-- `set key value`, so it applies immediately and is remembered next run.

local function opt_set(key, value) wlens.exec("set " .. key .. " " .. tostring(value)) end

-- An on/off switch for a boolean option. `on`/`off` map other option types
-- (e.g. filter: on = "linear", off = "nearest").
function W.toggle(key, o)
  o = o or {}
  local st = W.style
  local on_v, off_v = o.on, o.off
  if on_v == nil then on_v = true end
  if off_v == nil then off_v = false end
  local function is_on() return wlens.opt[key] == on_v end
  return {
    width = 38, height = 22, padding = 3, radius = 11,
    background = wlens.color(st.fg, 0.18),
    selected = { when = is_on, background = st.accent, justify = "end" },
    on_click = function()
      if is_on() then opt_set(key, off_v) else opt_set(key, on_v) end
    end,
    { width = 16, height = 16, radius = 8, pass_through = true,
      background = st.fg, selected = { when = is_on, background = st.bg } },
  }
end

-- Segmented buttons for an option with a few values:
--   W.choice("fit", { { "Shrink", "shrink" }, { "Contain", "contain" } })
function W.choice(key, options, o)
  local st = W.style
  local row = { dir = "row", wrap = true, gap = 2, padding = 2, radius = st.button_radius,
                background = wlens.color(st.fg, 0.07) }
  for _, opt in ipairs(options) do
    local label, value = opt[1], opt[2]
    local function cur() return tostring(wlens.opt[key]) == tostring(value) end
    row[#row + 1] = {
      text = label, padding = { 4, 10 }, radius = math.max(2, st.button_radius - 2),
      color = st.fg, hover = { background = wlens.color(st.fg, 0.1) },
      selected = { when = cur, background = st.accent, color = st.bg },
      on_click = function() if not cur() then opt_set(key, value) end end,
    }
  end
  return E(row, o or {})
end

-- A slider for a number option between `min` and `max`.
function W.range(key, min, max, o)
  o = o or {}
  local st = W.style
  return E({
    width = o.width or 180, height = 18, padding = { 6, 0 },
    slider = {
      value = function() return ((tonumber(wlens.opt[key]) or min) - min) / (max - min) end,
      on_change = function(v)
        local n = min + v * (max - min)
        if o.step then n = math.floor(n / o.step + 0.5) * o.step end
        opt_set(key, n)
      end,
      track = wlens.color(st.fg, 0.15), fill = st.accent,
    },
  }, o.extra or {})
end

-- A settings row: label (+ hint) on the left, a control on the right.
function W.setting(label, control, hint)
  local st = W.style
  local left = { dir = "column", gap = 2, align = "start", grow = true,
                 { text = label, color = st.fg, text_align = "left" } }
  if hint then left[#left + 1] = { text = hint, tooltip = hint, color = st.dim, font_size = 12, text_align = "left" } end
  return { dir = "row", wrap = true, width = "100%", gap = 16, padding = { 6, 0 }, left, control }
end

-- The settings dialog: tabs of settings rows. `tabs` is a list of
-- { "Title", { W.setting(...), ... } }; scripts may add their own tab.
-- Built when the dialog is (so it uses the current theme's colours).
-- Scripts add their own with W.add_settings_tab(title, function() return rows end).
W.extra_settings_tabs = {}
function W.add_settings_tab(title, rows) table.insert(W.extra_settings_tabs, { title, rows }) end

function W.default_settings_tabs()
  local tabs = {
  { "View", {
    W.setting("Fit", W.choice("fit", { { "Shrink", "shrink" }, { "Contain", "contain" }, { "Cover", "cover" }, { "Width", "width" } }),
              "How images fit the window"),
    W.setting("Smooth scaling", W.toggle("filter", { on = "linear", off = "nearest" }), "Off: crisp pixels when zoomed in"),
    W.setting("Checkerboard", W.toggle("image.checker"), "Behind transparent images"),
    W.setting("Wrap around", W.toggle("wrap"), "Go from the last image to the first"),
    W.setting("Small images", W.choice("pan", { { "Centred", "center" }, { "Movable", "bounded" }, { "Free", "free" } })),
    W.setting("Transition", W.choice("transition.kind", { { "None", "none" }, { "Fade", "fade" }, { "Slide", "slide" }, { "Zoom", "zoom" } })),
  } },
  { "Gallery", {
    W.setting("Thumbnails", W.choice("gallery.fit", { { "Crop", "cover" }, { "Fit", "contain" } })),
    W.setting("Names", W.toggle("gallery.labels"), "Under each thumbnail"),
    W.setting("Size", W.range("gallery.size", 96, 400, { step = 8 })),
  } },
  { "Video", {
    W.setting("When a video ends", W.choice("video_end", { { "Loop", "loop" }, { "Next", "next" }, { "Stop", "stop" } })),
  } },
}
  for _, t in ipairs(W.extra_settings_tabs) do
    tabs[#tabs + 1] = { t[1], type(t[2]) == "function" and t[2]() or t[2] }
  end
  return tabs
end

function W.settings(o)
  o = o or {}
  local st = W.style
  local tabs = o.tabs or W.default_settings_tabs()
  local current = 1
  local tabbar = { dir = "row", wrap = true, gap = 4, width = "100%" }
  local pages = { stack = true, width = "100%" }
  for i, tab in ipairs(tabs) do
    local function active() return current == i end
    local function select() current = i end
    tabbar[#tabbar + 1] = {
      text = tab[1], padding = { 6, 12 }, radius = st.button_radius, color = st.dim,
      hover = { background = st.hover, color = st.fg },
      selected = { when = active, background = wlens.color(st.accent, 0.18), color = st.accent },
      on_click = select,
    }
    local page = { dir = "column", width = "100%", gap = 2, align = "start", show = active, anim = 0 }
    for _, row in ipairs(tab[2]) do page[#page + 1] = row end
    pages[#pages + 1] = page
  end
  local panel = {
    dir = "column", gap = 14, padding = { 18, 22 }, width = o.width or 520, responsive = true, align = "start", max_height = "100%", scroll = true,
    background = st.panel, radius = st.radius + 4,
    border = { width = 1, color = st.panel_border },
    shadow = { size = 30, color = "#000000aa", y = 10 },
    { dir = "row", width = "100%",
      { text = "Settings", font_size = 17, weight = 600, grow = true, text_align = "left" },
      W.button(W.icons.close, "ui hide " .. (o.id or "settings"), nil, { font_size = 15 }) },
    tabbar,
    W.divider(),
    pages,
    { text = "Changes apply right away and are remembered.", color = st.dim, font_size = 12 },
  }
  return overlay(o.id or "settings", panel, { scrim = o.scrim or st.scrim })
end

-- Group bindings by what they do: { {keys = "n  space  Right", desc = "next"}, ... }
function W.binding_rows()
  local rows, by_desc = {}, {}
  for _, b in ipairs(wlens.bindings()) do
    if b.mode then b = { key = b.key, desc = b.mode .. ": " .. b.desc } end
    local r = by_desc[b.desc]
    if r then
      r.keys = r.keys .. "  " .. b.key
    else
      r = { keys = b.key, desc = b.desc }
      by_desc[b.desc] = r
      rows[#rows + 1] = r
    end
  end
  return rows
end

-- Keybinding cheat sheet, built live from wlens.bindings(). Uses as many
-- columns as fit the window (up to `columns`).
function W.help(o)
  o = o or {}
  local max_cols = o.columns or 4
  local col_w = o.column_width or 300
  local function fit()
    -- Fit the area it's shown over (the main view), else the window.
    local v = wlens.view()
    local win = wlens.window()
    local width = (v.width > 0) and v.width or win.width
    local height = (v.height > 0) and v.height or win.height
    local by_width = math.floor((width - 80) / col_w)
    -- Enough columns that one column fits the height (rows are ~15px).
    local per_col = math.max(1, math.floor((height - 140) / 15))
    local by_height = math.ceil(#W.binding_rows() / per_col)
    return math.max(1, math.min(max_cols, math.max(by_width, by_height)))
  end
  local function column(c, field)
    return function()
      local rows, cols = W.binding_rows(), fit()
      local per = math.ceil(#rows / cols)
      local out = {}
      for i = (c - 1) * per + 1, math.min(c * per, #rows) do
        out[#out + 1] = rows[i][field]
      end
      return table.concat(out, "\n")
    end
  end
  local grid = { gap = 28, align = "start" }
  for c = 1, max_cols do
    grid[#grid + 1] = {
      gap = 14, align = "start",
      show = function() return c <= fit() end,
      { text = column(c, "keys"), color = W.style.accent,
        font = o.key_font or "monospace", font_size = 12 },
      { text = column(c, "desc"), color = W.style.fg, font_size = 12 },
    }
  end
  local panel = {
    direction = "column", padding = { 16, 22 }, gap = 14,
    background = W.style.panel, radius = W.style.radius + 4,
    border = { width = 1, color = W.style.panel_border },
    shadow = { size = 30, color = "#000000aa", y = 10 },
    { width = "100%",
      { text = "Keybindings", font_size = 16, grow = true, text_align = "left" },
      W.button(W.icons.close, "ui hide " .. (o.id or "help"), nil, { font_size = 15 }) },
    grid,
  }
  return overlay(o.id or "help", panel, { scrim = o.scrim or W.style.scrim })
end

-- Click the left/right edge of the window to go back/forward. The zones step
-- aside while you're zoomed in, so clicking there pans instead.
--
--   w.nav_zones {
--     width = "15%",            -- how far in from each edge counts
--     indicator = "none",       -- "none":    invisible; just click (default)
--                               -- "chevron": a small button fades in near the edge
--                               -- "tint":    the whole zone lightens (+ chevron)
--     reveal = "hover",         -- when the chevron shows: "hover" (over its zone),
--                               -- "mouse" (while the mouse moves), "always"
--     size = 36, inset = 16,    -- chevron button size / distance from the edge
--     background = "#00000080", color = "#ffffffd0", tint = "#ffffff08",
--     pointer = true,           -- pointer cursor over the zone (even when invisible)
--   }
function W.nav_zones(o)
  o = o or {}
  local indicator = o.indicator or "none"
  local size = o.size or 36
  local function zone(side, action, icon)
    local z = {
      id = "nav_" .. action,
      width = o.width or "15%", height = "100%",
      justify = side == "left" and "start" or "end",
      padding = { 0, o.inset or 16 },
      on_click = action,
      -- Pointer only when there's somewhere to go. At the first/last image
      -- the zone stays (so a click there says "Last image" instead of
      -- falling through to the backdrop, which quits the lightbox).
      cursor = function(img)
        if o.pointer == false or not img then return "default" end
        local can = action == "prev" and img.has_prev or action == "next" and img.has_next
        return can and "pointer" or "default"
      end,
      -- Not while zoomed in (clicks pan) or in the gallery.
      show = function(img)
        return img ~= nil and img.mode ~= "gallery" and not img.zoomed_in
      end,
    }
    if indicator == "tint" then
      z.hover = { background = o.tint or wlens.color(W.style.fg, 0.04) }
    end
    if indicator ~= "none" then
      local chevron = {
        width = size, height = size, radius = W.style.pill_radius,
        text = icon, font = W.style.icon_font, font_size = math.floor(size * 0.62),
        on_click = action, -- clicks on the button itself still navigate
      }
      local reveal = o.reveal or "hover"
      local function can(img)
        return img ~= nil and (action == "prev" and img.has_prev or action == "next" and img.has_next)
      end
      if reveal == "hover" then
        chevron.follow_hover = true
        chevron.opacity = 0
        chevron.hover = { opacity = 1 }
      elseif reveal == "mouse" then
        chevron.show = function(img) return can(img) and "hover" or false end
      end
      if reveal ~= "mouse" then
        chevron.show = function(img) return can(img) end -- nothing to point at at the ends
      end
      if indicator == "chevron" then
        chevron.background = o.background or W.style.panel
        chevron.border = { width = 1, color = W.style.panel_border }
      end
      chevron.color = o.color or W.style.fg
      z[1] = chevron
    end
    return z
  end
  -- `wlens.ui` takes a list; return both zones as one transparent group.
  return {
    width = "100%", height = "100%",
    zone("left", "prev", W.icons.prev),
    { grow = true },
    zone("right", "next", W.icons.next),
  }
end

-- Browse folders as levels: subfolders are containers (covered by their
-- first image) and opening one goes inside; Back/Escape comes out again.
--   wlens.widgets.browse("/home/me/Pictures")
function W.browse(path, o)
  o = o or {}
  path = path:gsub("/+$", "")
  if path == "" then path = "/" end
  local ok, entries = pcall(wlens.list_dir, path)
  if not ok then return wlens.toast("Can't open " .. path, 4) end
  local items = {}
  for _, e in ipairs(entries) do
    if e.dir then
      -- Peek one level in for a cover and a count.
      local cover, count = nil, 0
      local ok2, inner = pcall(wlens.list_dir, e.path)
      if ok2 then
        for _, f in ipairs(inner) do
          if f.image then
            count = count + 1
            cover = cover or f.path
          end
        end
      end
      items[#items + 1] = {
        src = e.path, label = e.name, container = true,
        thumb = cover, count = count > 0 and count or nil, icon = W.icons.folder,
        open = function() W.browse(e.path, { push = true }) end,
      }
    elseif e.image then
      items[#items + 1] = { src = e.path }
    end
  end
  -- Folders first, like a file manager (stable, so names stay sorted).
  local folders, files = {}, {}
  for _, it in ipairs(items) do
    if it.container then folders[#folders + 1] = it else files[#files + 1] = it end
  end
  items = folders
  for _, it in ipairs(files) do items[#items + 1] = it end
  -- Folder names matter here, so show labels unless asked not to.
  -- A default, not a remembered change (wlens.theme, not wlens.set).
  if o.labels ~= false then wlens.theme { gallery = { labels = true } } end
  local title = path:match("([^/]+)$") or path
  if o.push then
    wlens.push_items(items, { title = title })
  else
    wlens.set_items(items)
    wlens.gallery("open")
  end
end

-- Transient messages: wlens.toast("Copied path")
W._toast = ""
local toast_timer
function W.toast(o)
  o = o or {}
  return {
    id = "toast", show = "never", layer = "top",
    anchor = o.position or "top", y = o.y or 18,
    padding = { 7, 14 }, radius = W.style.pill_radius,
    background = o.background or W.style.panel,
    border = { width = 1, color = W.style.panel_border },
    text = function() return W._toast end,
    color = W.style.fg,
  }
end

function wlens.toast(msg, secs)
  W._toast = msg
  if wlens.visible("toast") == nil then
    print("wlens: " .. msg) -- no toast widget in this UI
    return
  end
  wlens.show("toast")
  if toast_timer then wlens.cancel(toast_timer) end
  toast_timer = wlens.after(secs or 1.6, function() wlens.hide("toast") end)
end

-- Slideshow: W.slideshow() toggles, W.slideshow(5) starts with 5s per image.
local slide
function W.slideshow(secs)
  if slide and not secs then
    wlens.cancel(slide)
    slide = nil
    wlens.toast("Slideshow off")
    return
  end
  if slide then wlens.cancel(slide) end
  secs = secs or W.slide_secs or 4
  local function tick()
    wlens.next()
    slide = wlens.after(secs, tick)
  end
  slide = wlens.after(secs, tick)
  wlens.toast(string.format("Slideshow: %gs per image", secs))
end

return W
