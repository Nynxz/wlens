-- Gruvbox: warm retro browns and oranges.
-- A built-in palette theme (copy it to ~/.config/wlens/themes/ to edit). Use it with
--   wlens --theme gruvbox             (on the `simple` layout, set by `base` below)
--   wlens --theme lightbox --palette gruvbox
--   wlens.use("gruvbox")  /  wlens.use("simple", { palette = "gruvbox:light" })
return {
  name = "Gruvbox",
  base = "simple",
  dark = {
    background = "#282828",
    foreground = "#ebdbb2",
    card = "#32302f",
    popover = "#32302f",
    popover_foreground = "#ebdbb2",
    primary = "#fe8019",
    ring = "#fe8019",
    accent = "#504945",
    border = "#504945",
    muted = "#3c3836",
    muted_foreground = "#a89984",
    sidebar = "#1d2021",
    radius = "0.3rem",
  },
  light = {
    background = "#fbf1c7",
    foreground = "#3c3836",
    card = "#f2e5bc",
    popover = "#f2e5bc",
    popover_foreground = "#3c3836",
    primary = "#af3a03",
    ring = "#af3a03",
    accent = "#d5c4a1",
    border = "#d5c4a1",
    muted = "#ebdbb2",
    muted_foreground = "#7c6f64",
    sidebar = "#f2e5bc",
    radius = "0.3rem",
  },
}
