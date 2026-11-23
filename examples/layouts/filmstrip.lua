-- Filmstrip: a photo-editor layout. A bar with the file name on top, an info panel on
-- the right, and a strip of neighbouring thumbnails along the bottom
-- (click one to jump to it).
--
--   wlens -c examples/layouts/filmstrip.lua ~/Pictures
local w = wlens.widgets
wlens.use("minimal", { palette = "tokyo-night" }) -- a clean slate in these colours
local s = w.style

wlens.theme {
  font = { size = 13 },
  image = { margin = 24, radius = 6, shadow = { size = 24, color = "#00000080", y = 6 } },
  transition = { kind = "fade", duration = 0.12 },
  anim = { view = 0.12 },
}

local THUMB_W, THUMB_H, RING, GAP = 112, 70, 2, 8
local MAX_THUMBS = 15

-- Thumbnails around the current item, as many as fit the window (an odd
-- number, so the current one is in the middle). Rebuilt when the item
-- changes or the window is resized.
local function strip(img)
  if not w.viewing(img) then return {} end
  local items = wlens.items()
  local fit = math.floor((wlens.window().width - 24 + GAP) / (THUMB_W + 2 * RING + GAP))
  local n = math.max(1, math.min(MAX_THUMBS, #items, fit))
  if n % 2 == 0 and n > 1 then n = n - 1 end
  local first = math.max(1, math.min(img.index - n // 2, #items - n + 1))
  local out = {}
  for i = first, first + n - 1 do
    local it, here = items[i], i == img.index
    -- A frame around the picture: accent-coloured when it's the current one,
    -- so the ring sits around the image rather than under it.
    out[#out + 1] = {
      padding = RING, radius = 5 + RING,
      background = here and s.accent or "transparent",
      on_click = "goto " .. i, cursor = "pointer", tooltip = it.label or it.src:match("[^/]*$"),
      { width = THUMB_W, height = THUMB_H, radius = 5, image = it.thumb or it.src,
        fit = "contain", background = s.bg, -- the whole picture, letterboxed
        opacity = here and 1 or 0.5, follow_hover = true, hover = { opacity = 1 } },
    }
  end
  return out
end

-- A label above a value, for the info panel.
local function fact(label, value, show)
  return { direction = "column", align = "start", gap = 2, show = show,
    w.label(label, { color = s.dim, font_size = 11 }),
    w.label(value, { max_width = 200 }),
  }
end

-- Shown in the gallery too (for the selected tile), so opening the gallery
-- doesn't resize the grid's area.
local panel = {
  id = "details", -- `ui toggle details` (the ⓘ button, or i) hides / shows it
  direction = "column", align = "start", width = 232, height = "100%", gap = 18,
  padding = { 20, 16 }, background = s.bar_background,
  w.label("Details", { font_size = 15, weight = "semibold" }),
  fact("Name", "{name}", w.browsing),
  fact("Dimensions", "{width} × {height}", function(img) return img and img.width > 0 end),
  fact("File size", "{size}", function(img) return img and img.size > 0 end),
  fact("Folder", function(img) return img and img.dir:match("([^/]+)/*$") or "" end),
  fact("Zoom", "{zoom}%", w.viewing),
  fact("Position", "{index} of {count}"),
}

local title = w.bar {
  position = "top",
  left = { w.menu_button(), w.navigation_button(), w.level() },
  center = { w.filename { max_width = 640 } },
  right = { w.button(w.icons.rotate_right, "rotate 90", "Rotate"),
            w.button(w.icons.info, "ui toggle details", "Details panel"),
            w.help_button() },
}

wlens.ui(w.list(
  w.layout {
    top = { title },
    right = panel,
    bottom = { { width = "100%", justify = "center", gap = GAP, padding = { 8, 0 },
                 background = s.bar_background, children = strip,
                 show = w.viewing, anim = 0 } }, -- hides at once: no fade to resize through
    view = w.list(
      w.nav_zones { width = "12%" },
      w.menu { position = "top-left", margin = { 8, 8, 8, 8 } },
      w.help(),
      w.settings(),
      w.toast { y = 18 },
      w.loading()
    ),
  }
))

wlens.bind("i", "ui toggle details", "Details panel")
