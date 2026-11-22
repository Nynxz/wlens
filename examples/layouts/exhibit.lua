-- Exhibit: a gallery wall. A light backdrop, the image hung with a mat and
-- a shadow, and a museum-style caption beneath it.
--
--   wlens -c examples/layouts/exhibit.lua ~/Pictures
local w = wlens.widgets
wlens.use("minimal")

local wall, ink, soft = "#ece8e1", "#2b2724", "#8a837a"
w.style.fg, w.style.dim, w.style.panel = ink, soft, "#f7f4eff2"
w.style.hover, w.style.accent = "#0000000f", "#9a5b3c"

wlens.theme {
  window = { background = wall },
  font = { family = "serif", size = 14, color = ink },
  image = {
    margin = 70, radius = 0, checker = false,
    border = { width = 14, color = "#fbfaf7" },
    shadow = { size = 34, color = "#3a2f2440", y = 14 },
  },
  gallery = { background = "#fbfaf7", selected = ink, hover = "#00000014", radius = 0,
              label_color = soft, padding = 40, gap = 18 },
  transition = { kind = "fade", duration = 0.25, easing = "ease_in_out" },
}

local caption = {
  anchor = "bottom", y = 0, height = 110, width = "100%",
  direction = "column", justify = "center", gap = 4, reserve = true,
  show = w.viewing,
  { text = function(img)
      -- "mountains-01.jpg" -> "Mountains 01"
      local n = (img and img.name or ""):gsub("%.%w+$", ""):gsub("[-_]", " ")
      return (n:gsub("^%l", string.upper))
    end, font_size = 22, color = ink },
  { text = "{width} × {height} px  ·  digital print", color = soft, font_size = 13 },
  { text = function(img) return img and ("No. " .. img.index .. " of " .. img.count) or "" end,
    color = soft, font_size = 12 },
}

wlens.ui(w.list(
  caption,
  w.layout {
    view = w.list(
      w.nav_zones { width = "20%", indicator = "chevron", reveal = "hover" },
      w.help(),
      w.toast { y = 18 },
      w.loading()
    ),
  }
))
