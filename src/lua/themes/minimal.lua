-- minimal: just the image. No UI, no animation. Also the clean starting
-- point for a layout of your own: everything reset, colours from a palette
-- if you give one, then add your UI with wlens.ui.
--   wlens.use("minimal", { background = "#000000", checker = false })
--   wlens.use("minimal", { palette = "tokyo-night" })   -- then wlens.ui(...)
-- Returns the palette's colours (or nil), for themes built on it.
return function(o)
  local w = wlens.widgets
  wlens.reset_theme()
  w.reset()
  local pal = o.palette and w.try_palette(o.palette)
  wlens.theme {
    window = { background = o.background or (o.palette and w.style.bg) or "#000000" },
    font = { family = o.font or w.style.font or "sans-serif", color = w.style.fg },
    image = { checker = o.checker ~= false and (o.palette and w.style.checker or true) or false },
  }
  return pal
end
