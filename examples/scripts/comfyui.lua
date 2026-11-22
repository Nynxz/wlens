-- Browse ComfyUI (https://github.com/Comfy-Org/ComfyUI) outputs over its HTTP API.
--
-- Two sources:
--   * Asset library (preferred): every output ComfyUI has indexed, persistently.
--     Needs ComfyUI started with --enable-assets (it's off by default).
--     Loaded a page at a time: more arrives as you scroll near the end.
--   * /history (fallback): recent runs only — it's in memory and clears when
--     ComfyUI restarts.
--
-- Standalone:
--   WLENS_COMFY_URL=http://127.0.0.1:8188 wlens -c examples/scripts/comfyui.lua
--   WLENS_COMFY_GROUP=none       one flat grid instead of a tile per day
--   WLENS_COMFY_SOURCE=history   force recent runs (a tile per run, labelled by prompt)
--   WLENS_COMFY_WATCH=10         check every 10s for new outputs and refresh
-- F5 / the refresh button reloads, keeping your place.
--
-- As a module (copy to ~/.config/wlens/comfyui.lua):
--   local comfy = require("comfyui").setup { url = "http://127.0.0.1:8188", watch = 10 }
--   wlens.bind("C-c", function() comfy.open() end, "ComfyUI outputs")
--   comfy.hook()   -- wires F5 refresh, infinite scroll, level changes and `watch`
--   (the standalone block at the bottom does the same)
--
-- For ComfyUI on this machine you can also just browse its output folder:
--   wlens --browse ~/ComfyUI/output
--
-- API used, checked against ComfyUI master (app/assets/api, server.py):
--   GET /api/assets?tags_all=output&sort=created_at&order=desc&limit=N&offset=M
--       -> { assets = [{ id, name, display_name, mime_type, created_at, preview_url }], total, has_more }
--   GET /api/assets/{id}/content?disposition=inline      the file itself
--   GET /view?...&preview=webp;Q                          re-encoded smaller preview
--   GET /history?max_items=N                              recent runs
-- Not verified against a live server: the "output" tag name, and the exact
-- wrapping of /history outputs (both shapes are handled).

local M = {}

local IMAGE_EXT = { png = true, jpg = true, jpeg = true, webp = true, gif = true, bmp = true, tif = true, tiff = true }

local function urlencode(s)
	return (tostring(s):gsub("[^%w%-%._~/]", function(c)
		return string.format("%%%02X", c:byte())
	end))
end

local function abs(url)
	return url:sub(1, 1) == "/" and (M.url .. url) or url
end

local function preview(url)
	if not M.preview then
		return nil
	end
	return url .. (url:find("?", 1, true) and "&" or "?") .. "preview=" .. urlencode("webp;70")
end

local function index_of(list, src)
	for i, it in ipairs(list) do
		if it.src == src then
			return i
		end
	end
	return nil
end

function M.setup(o)
	o = o or {}
	M.url = (o.url or os.getenv("WLENS_COMFY_URL") or "http://127.0.0.1:8188"):gsub("/+$", "")
	M.source = o.source or os.getenv("WLENS_COMFY_SOURCE") or "auto" -- auto | assets | history
	M.group = o.group or os.getenv("WLENS_COMFY_GROUP") or "day" -- day | none
	M.page_size = o.page_size or 200 -- API max is 500
	M.max_runs = o.max_runs or 200
	M.preview = o.preview ~= false -- small webp previews in the grid
	M.watch = o.watch or tonumber(os.getenv("WLENS_COMFY_WATCH") or "")
	if o.headers then
		wlens.http_headers(M.url .. "/", o.headers)
	end -- e.g. an auth proxy
	return M
end

---------------------------------------------------------------- time

-- Seconds between local time and UTC right now.
local function utc_offset()
	local now = os.time()
	return os.difftime(now, os.time(os.date("!*t", now)))
end

-- ISO 8601 -> epoch seconds. Honours "Z" / "+hh:mm"; no suffix = UTC.
local function parse_iso(s)
	if type(s) ~= "string" then
		return nil
	end
	local y, mo, d, h, mi, se, rest = s:match("^(%d+)-(%d+)-(%d+)[T ](%d+):(%d+):(%d+)(.*)$")
	if not y then
		y, mo, d = s:match("^(%d+)-(%d+)-(%d+)")
		if not y then
			return nil
		end
		h, mi, se, rest = 0, 0, 0, ""
	end
	-- os.time() reads a table as local time; shift so the fields mean UTC.
	local t = os.time({
		year = tonumber(y),
		month = tonumber(mo),
		day = tonumber(d),
		hour = tonumber(h),
		min = tonumber(mi),
		sec = tonumber(se),
		isdst = false,
	}) + utc_offset()
	local sign, oh, om = rest:match("([%+%-])(%d%d):?(%d%d)%s*$")
	if sign then
		local off = tonumber(oh) * 3600 + tonumber(om) * 60
		t = t - (sign == "+" and off or -off)
	end
	return t
end

-- The asset's *local* calendar day.
local function day_key(a)
	local t = parse_iso(a.created_at)
	return t and os.date("%Y-%m-%d", t) or "unknown"
end

local function day_label(key)
	if key == os.date("%Y-%m-%d") then
		return "Today"
	end
	if key == os.date("%Y-%m-%d", os.time() - 86400) then
		return "Yesterday"
	end
	local y, m, d = key:match("^(%d+)-(%d+)-(%d+)")
	if not y then
		return key
	end
	local t = os.time({ year = tonumber(y), month = tonumber(m), day = tonumber(d), hour = 12 })
	return os.date(tonumber(y) == tonumber(os.date("%Y")) and "%b %d" or "%b %d %Y", t)
end

---------------------------------------------------------------- asset library

-- Loaded so far (newest first), plus paging state.
M.lib = { assets = {}, offset = 0, total = 0, has_more = true, loading = false }

-- Fetch the next page; cb(new_assets) or fail(code, status).
local function load_page(limit, cb, fail)
	local lib = M.lib
	if lib.loading then
		return
	end
	lib.loading = true
	local url = string.format(
		"%s/api/assets?tags_all=output&sort=created_at&order=desc&limit=%d&offset=%d",
		M.url,
		math.min(limit, 500),
		lib.offset
	)
	wlens.fetch(url, function(body, err, status)
		if M.lib ~= lib then return end -- a newer refresh owns the library
		lib.loading = false
		local ok, res = pcall(wlens.json.decode, body or "")
		if err or not ok or type(res) ~= "table" or type(res.assets) ~= "table" then
			local code = ok and type(res) == "table" and res.error and res.error.code
			return (fail or function()
				wlens.toast("ComfyUI: " .. tostring(code or err), 5)
			end)(code or err or "bad response", status)
		end
		lib.offset = lib.offset + #res.assets
		lib.total = res.total or lib.total
		lib.has_more = res.has_more and #res.assets > 0
		local new = {}
		for _, a in ipairs(res.assets) do
			if type(a.mime_type) == "string" and a.mime_type:match("^image/") then
				lib.assets[#lib.assets + 1] = a
				new[#new + 1] = a
			end
		end
		cb(new)
	end)
end

local function asset_item(a)
	return {
		src = M.url .. "/api/assets/" .. a.id .. "/content?disposition=inline",
		label = a.display_name or a.name,
		thumb = a.preview_url and preview(abs(a.preview_url)) or nil,
	}
end

-- Group loaded assets by local day. The oldest day may continue on the next
-- page, so it's marked partial ("120+") while more remain.
local function build_days()
	local days, order = {}, {}
	for _, a in ipairs(M.lib.assets) do
		local key = day_key(a)
		if not days[key] then
			days[key] = {}
			order[#order + 1] = key
		end
		table.insert(days[key], asset_item(a))
	end
	local items = {}
	for i, key in ipairs(order) do
		local imgs = days[key]
		local partial = i == #order and M.lib.has_more
		local key_src = M.url .. "/assets/day/" .. key -- unique key, never loaded
		items[#items + 1] = {
			src = key_src,
			label = day_label(key),
			container = true,
			thumb = imgs[1].thumb or imgs[1].src,
			count = partial and (#imgs .. "+") or tostring(#imgs),
			icon = wlens.widgets.icons.calendar,
			open = function()
				M.state.day, M.state.day_src = key, key_src
				wlens.push_items(imgs, { title = day_label(key) })
			end,
		}
	end
	return items, days, order
end

local function asset_items_flat()
	local items = {}
	for _, a in ipairs(M.lib.assets) do
		items[#items + 1] = asset_item(a)
	end
	return items
end

-- The top level for the current grouping.
local function top_items()
	if M.group == "none" then
		return asset_items_flat()
	end
	return (build_days())
end

---------------------------------------------------------------- /history

local function view_url(img)
	return string.format(
		"%s/view?filename=%s&subfolder=%s&type=%s",
		M.url,
		urlencode(img.filename),
		urlencode(img.subfolder or ""),
		urlencode(img.type or "output")
	)
end

local function run_images(entry)
	local out, nodes = {}, {}
	for node_id in pairs(entry.outputs or {}) do
		nodes[#nodes + 1] = node_id
	end
	table.sort(nodes, function(a, b)
		return (tonumber(a) or 0) < (tonumber(b) or 0)
	end)
	for _, node_id in ipairs(nodes) do
		local o = entry.outputs[node_id]
		o = (type(o) == "table" and o.output) or o -- tolerate { meta, output } wrapping
		for _, key in ipairs({ "images", "gifs", "animated" }) do
			for _, img in ipairs(type(o) == "table" and type(o[key]) == "table" and o[key] or {}) do
				local ext = type(img) == "table" and img.filename and img.filename:match("%.(%w+)$")
				if ext and IMAGE_EXT[ext:lower()] and (img.type or "output") == "output" then
					out[#out + 1] = img
				end
			end
		end
	end
	return out
end

local function prompt_text(entry)
	local best
	local function scan(t, depth)
		if type(t) ~= "table" or depth > 3 then
			return
		end
		for _, v in pairs(t) do
			if type(v) == "table" then
				local text = v.inputs and v.inputs.text
				if type(v.class_type) == "string" and v.class_type:find("TextEncode") and type(text) == "string" then
					if not best or #text > #best then
						best = text
					end
				else
					scan(v, depth + 1)
				end
			end
		end
	end
	scan(entry.prompt, 0)
	return best
end

local function started_at(entry)
	for _, m in ipairs((entry.status or {}).messages or {}) do
		local event, data = m[1] or m.event, m[2] or m.data
		if event == "execution_start" and type(data) == "table" and data.timestamp then
			return math.floor(data.timestamp / 1000)
		end
	end
end

local function short(s, n)
	s = s:gsub("%s+", " ")
	return #s > n and (s:sub(1, n - 1) .. "…") or s
end

local function history_view(hist)
	local runs = {}
	for id, entry in pairs(hist) do
		runs[#runs + 1] = { id = id, entry = entry, t = started_at(entry) or 0 }
	end
	table.sort(runs, function(a, b)
		return a.t > b.t
	end)
	local items, by_run = {}, {}
	for _, run in ipairs(runs) do
		local imgs = {}
		for _, img in ipairs(run_images(run.entry)) do
			imgs[#imgs + 1] = { src = view_url(img), label = img.filename, thumb = preview(view_url(img)) }
		end
		if #imgs > 0 then
			local text = prompt_text(run.entry)
			local when = run.t > 0 and os.date("%b %d %H:%M", run.t) or nil
			local label = text and short(text, 40) or when or ("Run " .. run.id:sub(1, 8))
			local title = when and (when .. " · " .. label) or label
			by_run[run.id] = imgs
			local key_src = M.url .. "/history/" .. run.id -- unique key, never loaded
			items[#items + 1] = {
				src = key_src,
				label = label,
				container = true,
				thumb = imgs[1].thumb or imgs[1].src,
				count = tostring(#imgs),
				icon = wlens.widgets.icons.image,
				open = function()
					M.state.day, M.state.day_src = run.id, key_src
					wlens.push_items(imgs, { title = title })
				end,
			}
		end
	end
	return items, by_run
end

local history_request
function M.fetch_history(cb)
	local token, depth, day = {}, wlens.depth(), M.state.day
	history_request = token
	wlens.fetch(M.url .. "/history?max_items=" .. M.max_runs, function(body, err)
		if history_request ~= token or wlens.depth() ~= depth or M.state.day ~= day then return end
		if err then
			return wlens.toast("ComfyUI: " .. err .. " (is it running?)", 5)
		end
		local ok, hist = pcall(wlens.json.decode, body)
		if not ok or type(hist) ~= "table" then
			return wlens.toast("ComfyUI: unexpected response", 5)
		end
		cb(hist)
	end)
end

---------------------------------------------------------------- open / scroll / refresh

M.state = { day = nil, day_src = nil, top_dirty = false }

-- Replace the top level, keeping the selection (and scroll position).
local function set_top(items)
	local cur = wlens.current()
	wlens.set_items(items, cur and index_of(items, cur.path) or 1)
end

local function load_history(refreshing)
	M.using = "history"
	wlens.set_more(false) -- /history comes back whole
	M.fetch_history(function(hist)
		local items, groups = history_view(hist)
		if #items == 0 then
			return wlens.toast("ComfyUI: no image outputs in history", 4)
		end
		if refreshing and wlens.depth() > 0 and groups[M.state.day] then
			local cur, imgs = wlens.current(), groups[M.state.day]
			wlens.pop_items()
			wlens.set_items(items, index_of(items, M.state.day_src) or 1)
			wlens.push_items(imgs, { title = cur and cur.level, index = cur and index_of(imgs, cur.path) })
		elseif refreshing then
			set_top(items)
		else
			wlens.set_items(items)
			wlens.gallery("open")
		end
		wlens.toast(refreshing and "ComfyUI: refreshed" or string.format("ComfyUI: %d recent runs", #items))
	end)
end

local function fallback(code, status, refreshing)
	if M.source == "assets" then
		return wlens.toast("ComfyUI assets: " .. tostring(code), 6)
	end
	if not refreshing then
		local why = (code == "SERVICE_DISABLED" or status == 503)
				and "asset library is off (start ComfyUI with --enable-assets)"
			or ("no asset library (" .. tostring(code) .. ")")
		wlens.toast("ComfyUI: " .. why .. " — showing recent history", 6)
	end
	load_history(refreshing)
end

function M.open()
	if M.source == "history" then
		return load_history(false)
	end
	M.lib = { assets = {}, offset = 0, total = 0, has_more = true, loading = false }
	wlens.toast("ComfyUI: loading…")
	load_page(M.page_size, function()
		M.using = "assets"
		wlens.set_items(top_items())
		wlens.set_more(M.lib.has_more)
		wlens.gallery("open")
		wlens.toast(string.format("ComfyUI: %d outputs", M.lib.total))
	end, function(code, status)
		fallback(code, status, false)
	end)
end

-- Infinite scroll: wlens fires `end_reached` near the end of the list.
function M.load_more()
	if M.using ~= "assets" or not M.lib.has_more or M.lib.loading then
		return
	end
	load_page(M.page_size, function(new)
		if wlens.depth() == 0 then
			set_top(top_items())
			wlens.set_more(M.lib.has_more)
		elseif M.group ~= "none" and M.state.day then
			-- Inside a day: append whatever of the next page belongs to it.
			local more = {}
			for _, a in ipairs(new) do
				if day_key(a) == M.state.day then
					more[#more + 1] = asset_item(a)
				end
			end
			if #more > 0 then
				wlens.add_items(more)
			end
			M.state.top_dirty = true
			-- The whole page belonged to this day: keep going so it fills up.
			if #more == #new and M.lib.has_more then
				wlens.after(0, M.load_more)
			end
		end
	end)
end

-- Bring the day list up to date when we come back to it.
function M.on_level()
	if wlens.depth() == 0 and M.state.top_dirty and M.using == "assets" then
		M.state.top_dirty = false
		set_top(top_items())
	end
end

-- Refresh: reload as much as was loaded, keep the place.
function M.refresh()
	if M.using == "history" or M.source == "history" then
		return load_history(true)
	end
	local want = math.max(M.lib.offset, M.page_size)
	local depth, day = wlens.depth(), M.state.day
	local cur = wlens.current()
	M.lib = { assets = {}, offset = 0, total = 0, has_more = true, loading = false }
	local function step()
		load_page(math.min(want - M.lib.offset, 500), function()
			if M.lib.has_more and M.lib.offset < want then
				return step()
			end
			M.using = "assets"
			if wlens.depth() ~= depth or M.state.day ~= day then
				M.state.top_dirty = true
				M.on_level()
				return
			end
			local items, days = top_items(), nil
			if M.group ~= "none" then
				items, days = build_days()
			end
			if wlens.depth() > 0 and days and days[M.state.day] then
				local imgs = days[M.state.day]
				wlens.pop_items()
				wlens.set_items(items, index_of(items, M.state.day_src) or 1)
				wlens.push_items(imgs, { title = cur and cur.level, index = cur and index_of(imgs, cur.path) })
			else
				set_top(items)
			end
			wlens.toast("ComfyUI: refreshed")
		end, function(code, status)
			fallback(code, status, true)
		end)
	end
	step()
end

-- Poll cheaply (one asset / the history keys) and refresh only on change.
local last_seen
function M.start_watch(secs)
	local function check()
		if M.using == "assets" then
			wlens.fetch(M.url .. "/api/assets?tags_all=output&sort=created_at&order=desc&limit=1", function(body, err)
				local ok, res = pcall(wlens.json.decode, body or "")
				local newest = not err and ok and type(res) == "table" and res.assets and res.assets[1]
				local mark = newest and (newest.id .. ":" .. tostring(res.total)) or nil
				if mark and last_seen and mark ~= last_seen then
					M.refresh()
				end
				last_seen = mark or last_seen
			end)
		elseif M.using == "history" then
			wlens.fetch(M.url .. "/history?max_items=1", function(body, err)
				local mark = not err and body or nil
				if mark and last_seen and mark ~= last_seen then
					M.refresh()
				end
				last_seen = mark or last_seen
			end)
		end
		wlens.after(secs, check)
	end
	wlens.after(secs, check)
end

-- Wire the hooks wlens fires.
function M.hook()
	wlens.on("refresh", M.refresh) -- F5 / the refresh button
	wlens.on("end_reached", M.load_more)
	wlens.on("level", M.on_level)
	if M.watch then
		M.start_watch(M.watch)
	end
end

if not ... then
	M.setup()
	wlens.use("simple")
	wlens.theme({ gallery = { labels = true } })
	wlens.on("start", M.open)
	M.hook()
end

return M
