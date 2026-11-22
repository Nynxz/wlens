-- Strawberry Milk: soft pink, light.
-- A built-in palette theme (copy it to ~/.config/wlens/themes/ to edit). Use it with
--   wlens --theme strawberry-milk             (on the `lightbox` layout, set by `base` below)
--   wlens --theme lightbox --palette strawberry-milk
--   wlens.use("strawberry-milk")  /  wlens.use("simple", { palette = "strawberry-milk:light" })
return {
  name = "Strawberry Milk",
  base = "lightbox",
  light = {
    background = "#fdf2f7",
    foreground = "#3b2333",
    card = "#ffffff",
    popover = "#ffffff",
    popover_foreground = "#3b2333",
    primary = "#f49ac1",
    ring = "#e05597",
    accent = "#fbe3ee",
    border = "#f3d3e2",
    muted = "#fbe8f0",
    muted_foreground = "#8b6478",
    sidebar = "#fff6fa",
    radius = "0.5rem",
    font_sans = "'Nunito Sans', 'Nunito', system-ui, sans-serif",
  },
}
