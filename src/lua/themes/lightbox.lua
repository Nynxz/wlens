-- lightbox: the image floats on a translucent backdrop (the compositor blurs
-- what's behind it), with rounded corners, a shadow, animated transitions,
-- click-the-edges navigation, and controls that fade in when you move the mouse.
--   wlens.use("lightbox", { backdrop = "#000000d0", transition = "slide",
--                           nav = { width = "25%", indicator = "tint" }, fullscreen = false })
-- With a palette, the backdrop, controls and corners follow it:
--   wlens.use("lightbox", { palette = "gruvbox" })
return function(o)
  local w = wlens.widgets
  local c = wlens.color
  local pal = wlens.use("minimal", { palette = o.palette })
  local s = w.style
  -- Without a palette: the original near-black glass look.
  local look = pal and {
    backdrop = c(s.bg, 0.78),
    pill = c(s.panel, 0.82),
    pill_border = s.panel_border,
    image_border = c(s.panel_border, 0.6),
    dim = s.dim,
    radius = math.max(6, s.radius * 1.5),
  } or {
    backdrop = "#08080cc8",
    pill = "#000000a0",
    pill_border = "#ffffff12",
    image_border = "#ffffff1c",
    dim = "#ffffff8c",
    radius = 12,
  }
  if not pal then
    s.fg = "#f2f2f2"
    s.hover = "#ffffff26"
    s.checker = { "#2a2a2e", "#36363b" }
  end

  wlens.theme {
    window = { background = o.backdrop or look.backdrop, transparent = true },
    image = {
      margin = o.margin or 56,
      radius = o.radius or look.radius,
      border = { width = 1, color = look.image_border },
      shadow = { size = 36, color = "#000000b8", y = 14 },
      checker = s.checker,
    },
    anim = { view = 0.18, ui = 0.2 },
    ui = { idle = 1.6 },
    transition = { kind = o.transition or "zoom", duration = 0.34, easing = o.easing or "ease_out" },
    font = { family = o.font or s.font or "sans-serif", size = 14, color = s.fg },
  }
  if not pal then
    -- lighter cells on the translucent backdrop
    wlens.theme { gallery = { background = "#ffffff12", selected = "#ffffffd0", radius = 10 } }
  end
  wlens.opt.fit = o.fit or "contain"

  -- Buttons sit 4px inside the pill: shrink their radius so corners nest.
  local inner = s.pill_radius >= 999 and 999 or math.max(2, s.pill_radius - 4)
  local pill = {
    background = look.pill, border = { width = 1, color = look.pill_border },
    shadow = { size = 12, color = "#00000080", y = 4 }, radius = s.pill_radius,
  }

  -- Sidebars (filled by scripts with wlens.slot) float as rounded panels.
  local panel = wlens.extend(pill, {
    dir = "column", align = "start", padding = 16, gap = 12, max_height = "100%",
    radius = math.min(s.pill_radius, 18), shadow = { size = 24, color = "#00000066", y = 6 },
  })

  local close = wlens.extend(pill, {
    anchor = "top-right", x = 18, y = 18, padding = 4, show = "hover", reserve = "gallery",
    w.button(w.icons.close, "quit", "Close", { font_size = 18, radius = inner }),
  })
  local caption = wlens.extend(pill, {
    id = "caption", anchor = "bottom", y = 16, padding = { 4, 6 }, gap = 6, show = "hover",
    reserve = "gallery", -- the grid scrolls under it but its last row clears it
    w.menu_button { radius = inner },
    w.back_button { radius = inner },
    w.refresh_button { radius = inner },
    w.gallery_button { radius = inner },
    w.filename { radius = inner },
    w.play_button { radius = inner },
    w.scrubber { width = 220, inset = 6, extra = { id = "caption_scrubber" } },
    w.timecode { padding = { 0, 6 } },
    w.volume { width = 56 },
    w.counter { color = look.dim, padding = { 0, 6 } },
    w.slot("caption", { gap = 6 }),
    o.help_button ~= false and w.help_button { radius = inner } or nil,
  })

  wlens.ui(w.list(
    -- Scripts' sidebars go left/right; the controls float over the main view
    -- (so they centre on it, not on the window, when a sidebar is open).
    w.layout {
      top = w.slot("top", { dir = "column", width = "100%" }, { padding = { 18, 18, 0, 18 }, width = "100%" }),
      left = w.slot("left", panel, { padding = 18, height = "auto", max_height = "100%" }),
      right = w.slot("right", panel, { padding = 18, height = "auto", max_height = "100%" }),
      view = w.list(
        o.nav_zones ~= false and w.nav_zones(wlens.extend({ width = "22%" }, o.nav or {})),
        close,
        caption,
        w.menu { position = "bottom-center", margin = { 8, 8, 64, 8 } },
        w.help(),
        w.settings(),
        -- Scripts' menus / dialogs, also over the main view.
        w.slot("overlay", { width = "100%", height = "100%" }),
        w.toast(),
        w.loading()
      ),
    }
  ))

  -- Clicking empty space only closes an open menu/overlay (never the app).
  wlens.bind("ClickBackdrop", function() w.dismiss() end, "Close menu")
  if o.fullscreen ~= false then
    -- "on", not a toggle: safe with -f or if the theme is applied twice.
    wlens.on("start", function() wlens.fullscreen("on") end)
  end
end
