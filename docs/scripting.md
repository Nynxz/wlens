# Scripting

Everything is exposed on the global `wlens` table. Lua runs on the viewer's
main thread; callbacks are short and queue work (commands, list changes,
HTTP requests) that the viewer carries out after they return.

## The current item

`wlens.current()` returns a table (or `nil`), also passed to hooks and to UI
`text`/`show`/`cursor` functions:

| Field | |
|---|---|
| `path`, `name`, `dir` | Path or URL; label or file name; folder / URL prefix |
| `index`, `count` | 1-based position, list length |
| `width`, `height`, `size` | Pixels (0 if unknown); file size in bytes |
| `zoom`, `fitted`, `zoomed_in` | View state |
| `has_prev`, `has_next` | Whether there's somewhere to go (respects `wrap`) |
| `mode` | `"image"`, `"video"` or `"gallery"` |
| `remote`, `container`, `count_in` | URL item; container; its badge text |
| `meta` | The item's `meta` table, if a script gave it one. In `{}` templates: `{meta.title}` |
| `depth`, `level` | Level depth and title |
| `loading` | The current image / video is still on its way (themes show a spinner) |
| `frames`, `frame` | Animated images: frame count (0 for stills), current frame |
| `video`, `position`, `duration`, `paused`, `volume`, `muted` | Video playback (seconds; volume 1.0 = 100%). In `{}` templates `position`/`duration` read as `m:ss`, `progress` is 0–1 |

## Functions

**Actions and settings**

| | |
|---|---|
| `next(n)`, `go(n)`, `zoom("in")`, `gallery("open")`, `toggle(id)`… | Every [command](configuration.md#commands) as a function: `wlens.video("seek", "+10")` runs `video seek +10`. `go` is `goto`; `show`/`hide`/`toggle`/`focus`/`blur(id)` are the `ui` commands (`wlens.ui` builds the UI instead, and `w.toggle` is a settings switch); `set` is `wlens.set` below |
| `exec(command)` | Run a command string |
| `bind(key, action [, description [, mode]])`, `unbind_all()`, `bindings()` | Keys. `bindings()` lists `{ mode, key, desc, command }` |
| `set(key, value)`, `get(key)`, `opt.key` | Options and theme properties |
| `theme{…}`, `reset_theme()`, `use(name, opts)` | [Theming](theming.md) |
| `ui{…}`, `visible(id)`, `widgets` | [UI](ui.md) |
| `title(text)`, `toast(msg [, secs])` | |
| `color(c [, alpha])` | Normalise a colour to `#rrggbbaa` |
| `mode()`, `depth()`, `window()` | Current mode; level depth; window size (logical px) |

**Items**

| | |
|---|---|
| `items()` | The current list: `{ {src, label, thumb, container, count, meta}, … }` |
| `set_items(list [, start])`, `add_items(list)` | Replace / extend it |
| `push_items(list [, {title, index}])`, `pop_items()` | Enter / leave a level. `wlens.depth()` reflects them immediately, though the list changes apply after your function returns. Pushing an empty list plus `set_more(true)` opens a level at once with a spinner, to fill when data arrives |
| `set_more(bool)` | This list may continue: shows a "Loading more…" row (a spinner while the list is still empty), and stepping past the end waits for the next page. Ordered with `set_items` / `push_items` / `pop_items`, so it applies to the list they leave you on |
| `view()` | `{x, y, width, height}` of the main view (image / grid area), logical px — e.g. to size a dialog to it; `window()` is the whole window |
| `store.get(key)`, `store.set(key, value)` | This script's own remembered values (strings, numbers, booleans, tables), kept between runs — e.g. the last folder or sort order. `set(key, nil)` forgets |
| `input(id [, text])` | Get / set a text field's text |
| `slot(name, widget)`, `clear_slot(name)` | Add to / empty a theme's slot (see [UI](ui.md#regions-and-slots)) |
| `list_dir(path)` | `{ {name, path, dir, image, video, archive}, … }`, in natural name order |

**Time, processes, network**

| | |
|---|---|
| `after(secs, fn) → id`, `cancel(id)` | Timers |
| `spawn(shell_command)` | Run without waiting. `$WLENS_FILE` is the current file (for URLs, the downloaded copy; `$WLENS_URL` is the URL), plus `$WLENS_INDEX`, `$WLENS_COUNT`, and `$WLENS_SOCKET` (see [Controlling a running viewer](usage.md#controlling-a-running-viewer)) |
| `fetch(url, [opts,] fn(body, err, status, headers)) → id` | HTTP in the background. `opts`: `method`, `headers`, `body`, or `json = {…}` (encoded, POST by default), `redirect = false` to get a 3xx itself. `err` is nil for 2xx and 304; the body is passed either way; `headers` has lowercased names (e.g. `last-modified`) |
| `download(url, path [, {on_progress = fn(bytes, total), on_done = fn(err, bytes)}]) → id` | Save a URL to a file on a worker thread (`~` expanded, folders created, written to a temporary file and renamed when complete). Sends `http_headers` |
| `exists(path)`, `mkdir(path)` | For checking what's already downloaded and preparing folders |
| `http_headers(prefix, headers)` | Send headers (e.g. an API key) with every request — API calls, downloads, thumbnails — to the exact scheme, host and port in `prefix`, optionally restricted by its path prefix |
| `json.decode(s)`, `json.encode(v)` | JSON `null` decodes to `nil` |

**Other:** `wlens.script_dir` (the running script's folder), `wlens.features` (`video`: whether this build plays video), `wlens.config_file` (absolute path of the running config, for
loading files next to it), `wlens.args` (`theme`, `palette` from the command
line), `wlens.extend(a, b, …)` (shallow merge).

## Hooks

`wlens.on(event, fn)`; handlers get the current item table. `wlens.off(event, fn)` removes one.

| Event | When |
|---|---|
| `start` | The window is up |
| `image` | A new image (or a video's first frame) is displayed |
| `select` | The current item changed (any mode) |
| `mode` | Switched between image and gallery |
| `level` | Entered or left a level |
| `end_reached` | Scrolled or stepped near the end of the list (once per list length) — load more here for infinite scroll |
| `refresh` | The `refresh` command (F5, the refresh button) |

## Patterns

**Infinite scroll:**

```lua
local page, loading = 1, false
wlens.on("end_reached", function()
  if loading then return end
  loading = true
  page = page + 1
  wlens.fetch("https://example.com/photos?page=" .. page, function(body)
    loading = false
    local items = parse(body)          -- your function: body -> items
    wlens.add_items(items)
    wlens.set_more(#items > 0)         -- "Loading more…" until a page comes back empty
  end)
end)
```

**Refresh keeping your place:**

```lua
wlens.on("refresh", function()
  local cur = wlens.current()
  load_items(function(items)           -- your function that fetches the list
    local keep = 1
    for i, it in ipairs(items) do
      if cur and it.src == cur.path then keep = i end
    end
    wlens.set_items(items, keep)
  end)
end)
```
