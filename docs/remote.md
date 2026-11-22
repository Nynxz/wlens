# Remote sources

Items can be `http(s)://` URLs. They download on background threads into
`~/.cache/wlens/remote/` (named by the URL's MD5) and get thumbnails like
local files. Give items a `thumb` URL to use a server's own thumbnails in the
grid instead of downloading full images. Remote video items without a thumbnail
URL show a placeholder in the grid; opening them starts playback.

Scripts build galleries from web APIs with `wlens.fetch` + `wlens.json` (see
[Scripting](scripting.md)); `wlens.http_headers(prefix, …)` attaches
headers to API requests, image downloads and thumbnails. The scope matches the
exact scheme, host and port, plus the registered path prefix. For example,
`http://media/` never matches `http://media.evil.net/` or `http://media:9999/`.
Redirects re-evaluate registered headers for each destination; per-request
headers are dropped when the origin changes. HTTPS downgrades and cross-origin
redirects that would replay a body or unsafe method are rejected.

Downloads use connection/header deadlines and a 30-second read-idle timeout,
so a progressing transfer may take longer than 60 seconds.

Keep secrets out of scripts: read keys from an environment variable or a file
next to your config.

## Proxy

Everything wlens fetches — images, thumbnails, API calls, downloads and
streamed video — can go through a proxy (e.g. your VPN's):

```sh
WLENS_PROXY=http://127.0.0.1:8118 wlens …        # or socks5://127.0.0.1:1080
```

or `wlens.opt.proxy = "http://127.0.0.1:8118"` in a config. Without either,
the usual `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` variables are used.

## Caching

- Images from URLs are downloaded once into `~/.cache/wlens/remote/` and
  reused across sessions; thumbnails go into the shared freedesktop
  thumbnail cache. The cache is kept under `remote_cache_mb` (default 2 GiB),
  dropping the least recently used files at startup.
- Videos from URLs up to 128 MiB are fetched into the same cache before
  playing (one request), so replays and seeking are free; bigger ones (or of
  unknown size, e.g. full-length videos from a media server) stream.
- `wlens.download` copies from the cache when the file was already viewed.
- `wlens.fetch` (API calls) isn't cached; scripts can send
  `If-Modified-Since` themselves.

## Examples

### A JSON API — [`examples/scripts/web-gallery.lua`](../examples/scripts/web-gallery.lua)

Fetches a list of posts and shows them as a labelled gallery. `F5` reloads.

### ComfyUI — [`examples/scripts/comfyui.lua`](../examples/scripts/comfyui.lua)

```sh
WLENS_COMFY_URL=http://127.0.0.1:8188 wlens -c examples/scripts/comfyui.lua
```

Reads the **asset library** (start ComfyUI with `--enable-assets`): outputs
grouped by local day (`WLENS_COMFY_GROUP=none` for a flat grid), loaded a page
at a time as you scroll. A day that continues on the next page shows `N+`.
Without the asset library it falls back to `/history` (recent runs only,
labelled by prompt; `WLENS_COMFY_SOURCE=history` forces it). `F5` refreshes in
place; `WLENS_COMFY_WATCH=10` polls for new outputs. Videos are skipped for now.

For a local ComfyUI you can also just browse the folder:
`wlens --browse ~/ComfyUI/output`.
