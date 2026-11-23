-- simple: a status bar that reserves its own space, built from
-- wlens.widgets. Colours can come from a palette: { palette = "gruvbox" }. Rearrange it with the `layout` option:
--   wlens.use("simple", {
--     position = "top", accent = "#f5a97f", smooth = true, nav = { width = "20%", indicator = "none" },
--     layout = function(w) return {
--       left  = { w.filename() },
--       right = { w.zoom(), w.counter(), w.menu_button() },
--     } end,
--   })
-- `b` (`ui toggle panels`) hides the bar and any sidebars; `ui toggle bar`
-- just the bar.
return function(o)
  local w = wlens.widgets
  wlens.use("minimal", { palette = o.palette }) -- e.g. "gruvbox", "gruvbox:light"
  w.style.fg = o.foreground or w.style.fg
  w.style.accent = o.accent or w.style.accent
  w.style.bar_background = o.bar_background or w.style.bar_background
  w.style.bar_height = o.bar_height or w.style.bar_height

  wlens.theme {
    window = { background = o.background or w.style.bg },
    font = { family = o.font or w.style.font or "sans-serif", size = o.font_size or 13, color = w.style.fg },
    image = { checker = w.style.checker },
    anim = { view = o.smooth and 0.12 or 0 },
    transition = { kind = o.transition or "none", duration = 0.15 },
  }

  local layout = (o.layout or function(w) return {
    left = { w.menu_button(), w.navigation_button(),
             w.level(), w.filename() },
    right = { w.slot("bar"), w.play_button(), w.timecode(), w.volume(), w.dims(), w.size(), w.zoom(), w.counter(),
              w.refresh_button(), o.help_button ~= false and w.help_button() or nil },
  } end)(w)
  local top = o.position == "top"
  layout.position = o.position or "bottom"
  layout.show = o.bar == false and "never" or nil

  wlens.ui(w.list(
    -- Regions: the bar (with the video seek bar next to it) above or below
    -- the main view, and left/right slots for scripts' sidebars.
    w.layout {
      top = top and { w.bar(layout),
                      w.slot("top", { dir = "column", width = "100%" }) }
                or { w.slot("top", { dir = "column", width = "100%" }) },
      bottom = not top and { w.bar(layout) } or nil,
      -- Sidebars look like the bar; scripts only add content.
      left = w.slot("left", { dir = "column", align = "start", padding = { 16, 16 }, gap = 12, background = w.style.bar_background }),
      right = w.slot("right", { dir = "column", align = "start", padding = { 16, 16 }, gap = 12, background = w.style.bar_background }),
      -- Edge click zones cover the image area only, not the bar or sidebars.
      -- Menus, help and toasts float over it too, so they centre on the
      -- image area rather than the window.
      view = w.list(
        o.nav_zones ~= false and w.nav_zones(wlens.extend({ width = "15%" }, o.nav or {})),
        w.menu { position = top and "top-left" or "bottom-left", margin = { 8, 8, 8, 8 } },
        w.help(),
        w.settings(),
        w.slot("overlay", { width = "100%", height = "100%" }), -- scripts' menus / dialogs
        w.toast { y = 18 },
        -- The seek bar sits right on the bar's edge (flush, no gap): it floats
        -- over the bottom of the video, its track along the very edge.
        w.scrubber { height = 12, thickness = 4,
                     extra = { anchor = top and "top" or "bottom",
                               padding = top and { 0, 0, 8, 0 } or { 8, 0, 0, 0 } } },
        w.loading()
      ),
    }
  ))
end
