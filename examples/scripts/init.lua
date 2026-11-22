-- Example ~/.config/wlens/init.lua
-- Built-in defaults (src/lua/default.lua) are loaded first; this file overrides them.

-- 1. Pick a starting look: "minimal" | "simple" | "lightbox" (or your own,
--    see themes/ below). Every preset takes an options table.
wlens.use("simple", { accent = "#cba6f7", smooth = true })
-- wlens.use("lightbox", { transition = "slide", fullscreen = false })

-- 2. Tweak anything on top. Nested tables flatten to dotted keys, so this
--    is the same as `set window.background #1e1e2e` from a keybind.
wlens.theme {
  window = { background = "#1e1e2e" },
  image = {
    checker = { "#313244", "#45475a" },
    radius = 0,
  },
  font = { family = "JetBrainsMono Nerd Font", size = 13 },
  transition = { kind = "fade", duration = 0.15 },
}

-- 3. Behaviour options (same keys as the `set` command)
wlens.opt.fit = "shrink"        -- none|shrink|contain|cover|width|height
wlens.opt.filter = "linear"     -- linear|nearest (only affects zoomed-in pixels)
wlens.opt.wrap = true
wlens.opt.preload = 3

-- 4. Rearrange the bar from widgets (wlens.widgets). Each widget is just a
--    table-returning function; restyle with wlens.extend(widget, {...}).
-- wlens.use("simple", {
--   nav = { width = "20%" },          -- click the edges for prev/next (invisible by default)
--   -- nav = { indicator = "chevron" },       -- show a small chevron on hover; nav_zones = false removes them
--   layout = function(w) return {
--     left   = { w.menu_button(), w.filename() },   -- click the name to copy its path
--     center = { w.counter() },
--     right  = {
--       w.zoom(),
--       w.button(w.icons.rotate_left, "rotate -90", "Rotate left"),
--       w.button(w.icons.rotate_right, "rotate 90", "Rotate right"),
--       w.button("wall", function() wlens.spawn('swww img "$WLENS_FILE"') end, "Set wallpaper"),
--       w.help_button(),
--     },
--   } end,
-- })
--
-- Or build the whole UI yourself; elements are boxes with style keys
-- (background, radius, border, shadow, color, opacity), layout keys (anchor,
-- x, y, width, height, padding, gap, direction, align, justify, grow) and
-- behaviour keys (text, tooltip, on_click, show, hover, id, reserve).
-- wlens.ui {
--   { id = "bar", anchor = "top", width = "100%", height = 24, padding = {0, 8},
--     background = "#181825", reserve = true,
--     { text = "{name}", grow = true, text_align = "left", on_click = "copy path" },
--     { text = function(img) return img and img.video and "video" or "" end },
--   },
--   wlens.widgets.menu { position = "top-left", items = {
--     { "Copy path", "copy path" }, "-", { "Quit", "quit" } } },
-- }

-- 5. Keybinds: a command string, or a Lua function
-- The description is what `?` shows.
wlens.bind("C-r", "rotate 90", "Rotate")
wlens.bind("t", function()
  local kinds = { none = "fade", fade = "slide", slide = "zoom", zoom = "none" }
  wlens.set("transition.kind", kinds[wlens.get("transition.kind")])
end, "Next transition")
wlens.bind("C-w", function()
  wlens.spawn('swww img "$WLENS_FILE"')    -- $WLENS_FILE is the current file
end, "Set as wallpaper")

-- toggle between fitting and 1:1
wlens.bind("z", function()
  local img = wlens.current()
  if not img then return end
  if img.zoom == 1 then wlens.fit() else wlens.zoom("actual") end
end, "Fit / 100%")

-- hooks: "start", "image"
wlens.on("image", function(img)
  wlens.title(string.format("%s — %d/%d — %dx%d", img.name, img.index, img.count, img.width, img.height))
end)

-- 6. Your own theme: put this in ~/.config/wlens/themes/mine.lua and call
--    wlens.use("mine"). A theme is just a function of an options table.
--
--   return function(o)
--     wlens.use("minimal", { palette = o.palette })   -- start from a clean slate
--     wlens.theme { image = { radius = 8 } }
--     wlens.ui { ... }
--   end
