-- Example: build a gallery from a JSON API.
--   wlens -c examples/scripts/web-gallery.lua
--
-- wlens.fetch runs the request on a worker thread and calls back on the main
-- thread; wlens.json decodes the body; wlens.set_items replaces what you browse.
-- URL images download into ~/.cache/wlens/remote and get freedesktop
-- thumbnails like local files.

-- Replace with your API. It should return something like:
--   { "posts": [ { "title": "Sunset", "image": "https://example.org/a.jpg" }, ... ] }
local API = os.getenv("WLENS_API") or "http://127.0.0.1:8765/index.json"

wlens.use("lightbox", { fullscreen = false })
wlens.theme { gallery = { labels = true, size = 180 } }

local function load(url)
  wlens.toast("Loading…")
  wlens.fetch(url, function(body, err)
    if err then return wlens.toast("Fetch failed: " .. err, 4) end
    local ok, data = pcall(wlens.json.decode, body)
    if not ok then return wlens.toast("Not JSON", 4) end
    local items = {}
    for _, p in ipairs(data.posts or {}) do
      local src = p.image or p.url or p.file
      if src and not src:match("^https?://") then
        src = url:gsub("[^/]*$", "") .. src -- relative to the API URL
      end
      items[#items + 1] = { src = src, label = p.title }
    end
    wlens.set_items(items)
    wlens.gallery("open")
    wlens.toast(#items .. " images")
  end)
end

wlens.on("start", function() load(API) end)
wlens.bind("F5", function() load(API) end, "Reload from API")
