-- Tokyo Night: deep blue night with neon accents.
-- A built-in palette theme (copy it to ~/.config/wlens/themes/ to edit). Use it with
--   wlens --theme tokyo-night             (on the `simple` layout, set by `base` below)
--   wlens --theme lightbox --palette tokyo-night
--   wlens.use("tokyo-night")  /  wlens.use("simple", { palette = "tokyo-night:light" })
return {
  name = "Tokyo Night",
  base = "simple",
  dark = {
    background = "#1a1b26",
    foreground = "#c0caf5",
    card = "#1f2335",
    popover = "#1f2335",
    popover_foreground = "#c0caf5",
    primary = "#7aa2f7",
    ring = "#7aa2f7",
    accent = "#292e42",
    border = "#3b4261",
    muted = "#24283b",
    muted_foreground = "#737aa2",
    sidebar = "#16161e",
    radius = "0.5rem",
  },
  light = {
    background = "#e1e2e7",
    foreground = "#343b58",
    card = "#eaebf0",
    popover = "#eaebf0",
    popover_foreground = "#343b58",
    primary = "#2e7de9",
    ring = "#2e7de9",
    accent = "#c4c8da",
    border = "#a8aecb",
    muted = "#d5d6db",
    muted_foreground = "#6172b0",
    sidebar = "#d0d5e3",
    radius = "0.5rem",
  },
}
