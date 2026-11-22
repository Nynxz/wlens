# Galleries

## The thumbnail grid

`Return` (or the grid button) switches between the image and a grid of
thumbnails. Click or `Return` opens the selected item; arrows/`hjkl` move;
scroll or drag to move through it; `+`/`-` or `Ctrl+scroll` resize; `c`
switches crop/fit. Style it with the `gallery.*` [theme keys](theming.md).

Thumbnails are cached in `~/.cache/thumbnails/{normal,large,x-large,xx-large}`
following the freedesktop thumbnail specification (file named by the MD5 of
the file's URI, invalidated by modification time), so they're shared with
file managers. Only thumbnails for visible cells (plus two rows) are made,
nearest the selection first.

## Levels

An item can be a **container** — a folder, an album, a remote gallery.
Containers draw as stacked tiles with a cover and a count. Opening one runs
its `open` action, which usually enters a new **level**:

```lua
wlens.set_items {
  { src = "/pics/trip", label = "Trip", container = true,
    thumb = "/pics/trip/01.jpg", count = 42, icon = wlens.widgets.icons.folder,
    open = function()
      wlens.push_items(list_of_items, { title = "Trip" })
    end },
}
```

- `push_items(list, { title, index })` enters a level; `back` (Backspace,
  the back button) returns with the previous selection and scroll restored.
- `Esc` climbs out one step at a time: image → its grid → parent level → quit.
- In image view, next/prev skip containers; choosing one opens it.
- `wlens.depth()` is how deep you are; `{level}` in UI text is the title.
- `pop_items()` leaves a level in order with other list changes made in the
  same callback (useful for rebuilding both levels on refresh).

## Folders

`wlens ~/Pictures` opens the folder browser. Folders and archives appear first,
then images and supported videos. Return opens the selected tile. Alt+Left
and Alt+Right, or the mouse's back and forward buttons, revisit a folder with
its selection and scroll position. Opening a different folder starts a new
forward history.

Open an archive tile, or use `wlens --browse photos.zip`, to navigate its
folders one level at a time. Opening `wlens photos.zip` directly still reads all
images in natural page order. Videos and archives embedded inside an archive
are skipped.

## Items

What you browse is a list of items:

```lua
{ src = "/path/or/https://url", label = "Shown as {name}", thumb = "cover or server thumbnail",
  container = true, open = function() --[[ load its items ]] end, count = "120+", icon = "…",
  video = true,                       -- play as video even without a video extension
  meta = { title = "…", people = { { name = "…", image = "url" } }, tags = { "…" } } }
```

`meta` is free-form: `{meta.title}` in UI text reads it, and the details
panel (`w.details`) shows its picture, facts and lists as clickable chips.

A bare string is shorthand for `{ src = string }`; `url` or `path` work in place of `src`. See
[Scripting](scripting.md) for the functions that read and change the list,
and [Remote sources](remote.md) for URLs.
