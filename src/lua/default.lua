-- wlens built-in defaults. Loaded before ~/.config/wlens/init.lua, using the
-- exact same API, so anything here can be overridden (or wiped with
-- wlens.unbind_all()) from your own config.

local bind = wlens.bind

-- navigation
bind("n", "next")
bind("space", "next")
bind("Right", "next")
bind("l", "next")
bind("p", "prev")
bind("BackSpace", "prev")
bind("Left", "prev")
bind("h", "prev")
bind("g", "first")
bind("Home", "first")
bind("G", "last")
bind("End", "last")
bind("C-n", "next 10", "Forward 10")
bind("C-p", "prev 10", "Back 10")

-- zoom
bind("+", "zoom in")
bind("=", "zoom in")
bind("-", "zoom out")
bind("ScrollUp", "zoom in")
bind("ScrollDown", "zoom out")
bind("1", "zoom actual")
bind("2", "zoom 200%")
bind("3", "zoom 300%")

-- fit modes
bind("F", "fit", "Reapply current fit mode")
bind("W", "fit contain")
bind("w", "fit shrink")
bind("e", "fit width")
bind("E", "fit height")
bind("c", "fit cover")

-- pan
bind("S-Left", "pan 100 0")
bind("S-Right", "pan -100 0")
bind("S-Up", "pan 0 100")
bind("S-Down", "pan 0 -100")
bind("H", "pan 100 0")
bind("L", "pan -100 0")
bind("K", "pan 0 100")
bind("J", "pan 0 -100")

-- transform
bind(">", "rotate 90")
bind("<", "rotate -90")
bind("|", "flip h")
bind("_", "flip v")
bind("r", "reset")

-- misc
bind("a", "set filter toggle", "Smooth / pixelated")
bind("A", "set image.checker toggle", "Checkerboard")
-- Bars and sidebars together (the presets' layout regions), else a lone `bar`.
bind("b", function()
  wlens.toggle(wlens.visible("panels") ~= nil and "panels" or "bar")
end, "Hide bars and sidebars")
bind("f", "fullscreen")
bind("R", "reload", "Reload the image from disk")
bind("F5", "refresh", "Refresh")
bind("P", "animation toggle", "Play / pause animation")
bind("q", "quit")
bind("Escape", function()
  if wlens.widgets.dismiss() then return end
  -- Inside a folder/album, Escape goes back up to its grid before quitting.
  if wlens.depth() > 0 then wlens.gallery("open") else wlens.quit() end
end, "Close menu/help, up a level, or quit")

-- clipboard, overlays, slideshow
bind("y", "copy path", "Copy path")
bind("Y", "copy image", "Copy image")
bind("?", "ui toggle help", "Keybindings")
bind("F1", "ui toggle help", "Keybindings")
bind("m", "ui toggle menu", "Menu")
bind("s", function() wlens.widgets.slideshow() end, "Slideshow on/off")

-- mouse pseudo-keys: ScrollUp/Down, ClickImage, ClickBackdrop, DoubleClick,
-- RightClick, MiddleClick, MouseBack, MouseForward (drag with the left button pans)
-- Double-click: fitted -> 100% at the click point (200% if it already is
-- 100%), zoomed -> back to fit.
-- Video (mpv-style). Mode "video" falls back to image bindings, so n/p,
-- zoom and pan still work.
local function vbind(k, a, d) bind(k, a, d, "video") end
vbind("space", "video toggle", "Play / pause")
vbind("P", "video toggle", "Play / pause")
-- Arrows still go to the next/previous item; Shift/Ctrl+arrows seek.
vbind("S-Right", "video seek +5", "Forward 5s")
vbind("S-Left", "video seek -5", "Back 5s")
vbind("C-Right", "video seek +30", "Forward 30s")
vbind("C-Left", "video seek -30", "Back 30s")
vbind("Up", "video volume +10", "Volume up")
vbind("Down", "video volume -10", "Volume down")
vbind("M", "video mute", "Mute")
vbind("ClickImage", "video toggle", "Play / pause")
for i = 0, 9 do vbind(tostring(i), "video seek " .. (i * 10) .. "%", nil) end

-- Right-click goes backwards: the previous image, or up a level in the grid.
bind("RightClick", "prev", "Previous image")
bind(",", "ui toggle settings", "Settings")
bind("DoubleClick", function()
  local img = wlens.current()
  if not img then return end
  if not img.fitted then
    wlens.fit()
  elseif math.abs(img.zoom - 1) > 0.01 then
    wlens.zoom("actual")
  else
    wlens.zoom("200%")
  end
end, "Toggle fit / 100%")

-- gallery: Return toggles the thumbnail grid. Binds with a 4th argument
-- only apply in that mode and win over the global ones.
bind("Return", "gallery toggle", "Gallery")
local function gbind(key, action, desc) bind(key, action, desc, "gallery") end
gbind("Return", "gallery close", "Open selected")
gbind("space", "gallery close", "Open selected")
-- Escape always climbs "out": overlay -> parent level -> image -> quit.
gbind("Escape", function()
  if wlens.widgets.dismiss() then return end
  if wlens.depth() > 0 then return wlens.back() end
  local cur = wlens.current()
  if cur and not cur.container then wlens.gallery("close") else wlens.quit() end
end, "Back / close")
gbind("BackSpace", "back", "Back (parent level)")
gbind("RightClick", "back", "Back (parent level)")
-- The mouse's side "back" button: image -> its grid -> parent level, like a browser.
bind("MouseBack", "back", "Back")
gbind("c", "set gallery.fit toggle", "Thumbnails: crop / fit")
gbind("Left", "gallery left");   gbind("h", "gallery left")
gbind("Right", "gallery right"); gbind("l", "gallery right")
gbind("Up", "gallery up");       gbind("k", "gallery up")
gbind("Down", "gallery down");   gbind("j", "gallery down")
gbind("PageUp", "gallery pageup")
gbind("PageDown", "gallery pagedown")
gbind("+", "gallery bigger");    gbind("=", "gallery bigger")
gbind("-", "gallery smaller")
gbind("ScrollUp", "gallery scroll -1")
gbind("ScrollDown", "gallery scroll 1")
gbind("C-ScrollUp", "gallery bigger")
gbind("C-ScrollDown", "gallery smaller")

-- look: built-in presets are "minimal", "simple" and "lightbox"
wlens.use("simple")

-- window title: name, position and size
wlens.on("image", function(img)
  wlens.title(string.format("wlens - %s [%d/%d] %dx%d",
    img.name, img.index, img.count, img.width, img.height))
end)
