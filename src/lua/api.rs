//! The `wlens` Lua API: every function scripts can call is registered here.

use super::*;

impl Script {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let shared = Rc::new(RefCell::new(Shared::default()));
        let w = lua.create_table()?;

        // wlens.bind(chord, "command" | function [, description])
        let sh = shared.clone();
        w.set(
            "bind",
            lua.create_function(
                move |_,
                      (key, action, desc, mode): (
                    String,
                    LuaValue,
                    Option<String>,
                    Option<String>,
                )| {
                    let chord: Chord = key.parse().map_err(lua_err)?;
                    if let Some(m) = &mode
                        && !matches!(m.as_str(), "image" | "gallery" | "video")
                    {
                        return Err(lua_err(format!(
                            "bind: unknown mode '{m}' (image|gallery|video)"
                        )));
                    }
                    let slot = (mode.clone(), chord);
                    let (action, command) = match action {
                        LuaValue::String(s) => {
                            let text = s.to_str()?.to_string();
                            (Action::Command(text.parse().map_err(lua_err)?), Some(text))
                        }
                        LuaValue::Function(f) => (Action::Lua(f), None),
                        LuaValue::Nil => {
                            let mut sh = sh.borrow_mut();
                            if sh.bindings.remove(&slot).is_some() {
                                sh.binding_revision += 1;
                            }
                            return Ok(());
                        }
                        other => {
                            return Err(lua_err(format!(
                                "bind: expected string or function, got {}",
                                other.type_name()
                            )));
                        }
                    };
                    let mut sh = sh.borrow_mut();
                    sh.binding_revision += 1;
                    let order = sh.binding_revision;
                    let desc = desc
                        .or_else(|| command.clone())
                        .unwrap_or_else(|| "(lua)".into());
                    sh.bindings.insert(
                        slot,
                        Binding {
                            action,
                            key,
                            desc,
                            command,
                            order,
                        },
                    );
                    Ok(())
                },
            )?,
        )?;

        let sh = shared.clone();
        w.set(
            "_bindings_revision",
            lua.create_function(move |_, ()| Ok(sh.borrow().binding_revision))?,
        )?;

        // wlens.bindings() -> { {key = "n", desc = "next"}, ... } in definition order
        let sh = shared.clone();
        w.set(
            "bindings",
            lua.create_function(move |lua, ()| {
                let sh = sh.borrow();
                let mut all: Vec<(&Option<String>, &Binding)> =
                    sh.bindings.iter().map(|((m, _), b)| (m, b)).collect();
                all.sort_by_key(|(_, b)| b.order);
                let out = lua.create_table()?;
                for (mode, b) in all {
                    let t = lua.create_table()?;
                    t.set("mode", mode.as_deref())?;
                    t.set("key", b.key.as_str())?;
                    t.set("desc", b.desc.as_str())?;
                    t.set("command", b.command.as_deref())?;
                    out.push(t)?;
                }
                Ok(out)
            })?,
        )?;

        // wlens.after(seconds, fn) -> id ; wlens.cancel(id)
        let sh = shared.clone();
        w.set(
            "after",
            lua.create_function(move |_, (secs, f): (f64, Function)| {
                let mut sh = sh.borrow_mut();
                sh.next_timer += 1;
                let id = sh.next_timer;
                let at =
                    std::time::Instant::now() + std::time::Duration::from_secs_f64(secs.max(0.0));
                sh.timers.push(Timer { id, at, f });
                Ok(id)
            })?,
        )?;
        let sh = shared.clone();
        w.set(
            "cancel",
            lua.create_function(move |_, id: u64| {
                sh.borrow_mut().timers.retain(|t| t.id != id);
                Ok(())
            })?,
        )?;

        // wlens.window() -> { width, height } in logical px
        let sh = shared.clone();
        w.set(
            "window",
            lua.create_function(move |lua, ()| {
                let (w, h) = sh.borrow().window;
                let t = lua.create_table()?;
                t.set("width", w)?;
                t.set("height", h)?;
                Ok(t)
            })?,
        )?;

        // wlens.view() -> { x, y, width, height }: the image / grid area.
        let sh = shared.clone();
        w.set(
            "view",
            lua.create_function(move |lua, ()| {
                let (x, y, vw, vh) = sh.borrow().view;
                let t = lua.create_table()?;
                t.set("x", x)?;
                t.set("y", y)?;
                t.set("width", vw)?;
                t.set("height", vh)?;
                Ok(t)
            })?,
        )?;

        // wlens.visible(id) -> bool | nil
        let sh = shared.clone();
        w.set(
            "visible",
            // nil when no element has that id
            lua.create_function(move |_, id: String| Ok(sh.borrow().visible.get(&id).copied()))?,
        )?;

        // wlens.items() -> { {src = "...", label = "..."}, ... }
        let sh = shared.clone();
        w.set(
            "items",
            lua.create_function(move |lua, ()| {
                let out = lua.create_table()?;
                for it in &sh.borrow().items {
                    let t = lua.create_table()?;
                    t.set("src", it.src.as_string())?;
                    t.set("label", it.label.as_deref())?;
                    t.set("container", it.container)?;
                    t.set("count", it.count.as_deref())?;
                    t.set("meta", it.meta.clone())?;
                    t.set("thumb", it.thumb.as_ref().map(|s| s.as_string()))?;
                    out.push(t)?;
                }
                Ok(out)
            })?,
        )?;

        // wlens.set_items(list [, start_index]) / wlens.add_items(list)
        // Entries are "path-or-url" strings or { src = ..., label = ... }.
        for (name, append) in [("set_items", false), ("add_items", true)] {
            let sh = shared.clone();
            w.set(
                name,
                lua.create_function(move |_, (list, start): (Table, Option<usize>)| {
                    let items = parse_items(&list)?;
                    let start = start.map(|i| i.saturating_sub(1));
                    let op = if append {
                        ItemsOp::Add(items)
                    } else {
                        ItemsOp::Set(items, start)
                    };
                    sh.borrow_mut().pending_items.push(op);
                    Ok(())
                })?,
            )?;
        }

        // wlens.push_items(list, { title = "Holiday", index = 1 }) — enter a level.
        let sh = shared.clone();
        w.set(
            "push_items",
            lua.create_function(move |_, (list, opts): (Table, Option<Table>)| {
                let items = parse_items(&list)?;
                let (mut start, mut title) = (None, None);
                if let Some(o) = opts {
                    start = o
                        .get::<Option<usize>>("index")?
                        .map(|i| i.saturating_sub(1));
                    title = o.get("title")?;
                }
                let mut s = sh.borrow_mut();
                s.pending_items.push(ItemsOp::Push(items, start, title));
                // wlens.depth() counts it straight away (it's applied later).
                s.depth += 1;
                Ok(())
            })?,
        )?;

        // wlens.pop_items() — leave the current level. Unlike exec("back"),
        // this is ordered with set/push calls made in the same callback.
        let sh = shared.clone();
        w.set(
            "pop_items",
            lua.create_function(move |_, ()| {
                let mut s = sh.borrow_mut();
                s.pending_items.push(ItemsOp::Pop);
                s.depth = s.depth.saturating_sub(1);
                Ok(())
            })?,
        )?;

        // wlens.color("oklch(0.7 0.1 30)" [, alpha]) -> "#rrggbbaa" (alpha 0..1 replaces it)
        w.set(
            "color",
            lua.create_function(|_, (c, alpha): (String, Option<f32>)| {
                let mut col = crate::model::theme::Color::parse(&c).map_err(lua_err)?;
                if let Some(a) = alpha {
                    col = col.with_alpha(a.clamp(0.0, 1.0));
                }
                Ok(col.to_string())
            })?,
        )?;

        // wlens.set_more(bool) — the current list may continue: the grid shows a
        // "Loading more…" row and going past the end waits for the next page.
        let sh = shared.clone();
        w.set(
            "set_more",
            lua.create_function(move |_, more: bool| {
                // Queued with set/push/pop, so it applies to the right list.
                sh.borrow_mut().pending_items.push(ItemsOp::More(more));
                Ok(())
            })?,
        )?;

        // wlens.depth() -> 0 at the top level
        let sh = shared.clone();
        w.set(
            "depth",
            lua.create_function(move |_, ()| Ok(sh.borrow().depth))?,
        )?;

        // wlens.list_dir(path) -> { {name, path, dir = bool, image = bool}, ... } sorted by name
        w.set(
            "list_dir",
            lua.create_function(|lua, path: String| {
                let mut entries: Vec<_> = std::fs::read_dir(&path)
                    .map_err(lua_err)?
                    .flatten()
                    .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                    .collect();
                entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
                let out = lua.create_table()?;
                for e in entries {
                    let p = e.path();
                    let t = lua.create_table()?;
                    t.set("name", e.file_name().to_string_lossy().as_ref())?;
                    t.set("path", p.to_string_lossy().as_ref())?;
                    t.set("dir", p.is_dir())?;
                    t.set("image", crate::files::is_image(&p))?;
                    t.set("video", crate::files::is_video(&p))?;
                    out.push(t)?;
                }
                Ok(out)
            })?,
        )?;

        // wlens.fetch(url, function(body, err) ... end) — async GET as text.
        let sh = shared.clone();
        w.set(
            "fetch",
            // wlens.fetch(url, fn) or wlens.fetch(url, opts, fn)
            //   opts = { method = "POST", headers = {...}, body = "..." | json = {...} }
            //   fn(body, err, status): err is nil on 2xx; body is passed either way.
            lua.create_function(move |lua, args: mlua::MultiValue| {
                let mut args = args.into_iter();
                let url = match args.next() {
                    Some(LuaValue::String(s)) => s.to_str()?.to_string(),
                    _ => return Err(lua_err("fetch: first argument must be a URL")),
                };
                let (opts, f) = match (args.next(), args.next()) {
                    (Some(LuaValue::Function(f)), None) => (None, f),
                    (Some(LuaValue::Table(o)), Some(LuaValue::Function(f))) => (Some(o), f),
                    _ => {
                        return Err(lua_err(
                            "fetch: usage fetch(url, [opts,] function(body, err, status))",
                        ));
                    }
                };
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(lua_err("fetch: only http(s) URLs are supported"));
                }
                let mut req = crate::io::remote::Request {
                    url,
                    method: "GET".into(),
                    headers: Vec::new(),
                    follow: true,
                    body: None,
                };
                if let Some(o) = opts {
                    // redirect = false: get the 3xx itself (and its Set-Cookie).
                    if o.get::<Option<bool>>("redirect")? == Some(false) {
                        req.follow = false;
                    }
                    if let Some(m) = o.get::<Option<String>>("method")? {
                        req.method = m;
                    }
                    if let Some(h) = o.get::<Option<Table>>("headers")? {
                        for pair in h.pairs::<String, String>() {
                            req.headers.push(pair?);
                        }
                    }
                    if let Some(b) = o.get::<Option<String>>("body")? {
                        req.body = Some(b);
                    }
                    let json: LuaValue = o.get("json")?;
                    if !json.is_nil() {
                        let j: serde_json::Value = lua.from_value(json)?;
                        req.body = Some(serde_json::to_string(&j).map_err(lua_err)?);
                        if !req
                            .headers
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        {
                            req.headers
                                .push(("Content-Type".into(), "application/json".into()));
                        }
                        if o.get::<Option<String>>("method")?.is_none() {
                            req.method = "POST".into();
                        }
                    }
                }
                let mut sh = sh.borrow_mut();
                sh.next_fetch += 1;
                let id = sh.next_fetch;
                sh.fetch_queue.push((id, req));
                sh.fetch_callbacks.insert(id, f);
                Ok(id)
            })?,
        )?;

        // wlens.download(url, path, { on_progress = fn(bytes, total), on_done = fn(err, bytes) })
        // Saves to `path` (`~` expanded, folders created) on a worker thread.
        let sh = shared.clone();
        w.set(
            "download",
            lua.create_function(
                move |_, (url, path, opts): (String, String, Option<Table>)| {
                    let (mut on_progress, mut on_done) = (None, None);
                    if let Some(o) = opts {
                        on_progress = o.get::<Option<Function>>("on_progress")?;
                        on_done = o.get::<Option<Function>>("on_done")?;
                    }
                    let mut s = sh.borrow_mut();
                    s.next_download += 1;
                    let id = s.next_download;
                    s.download_queue.push((id, url, expand_home(&path)));
                    s.download_callbacks.insert(id, (on_progress, on_done));
                    Ok(id)
                },
            )?,
        )?;

        // wlens.update_ui(): kept for old scripts; the UI now re-evaluates by
        // itself after any script code runs.
        let sh = shared.clone();
        w.set(
            "update_ui",
            lua.create_function(move |_, ()| {
                sh.borrow_mut().ui_dirty = true;
                Ok(())
            })?,
        )?;

        // wlens.exists(path) -> bool, wlens.mkdir(path): for checking what's
        // already downloaded and preparing folders (`~` expanded).
        w.set(
            "exists",
            lua.create_function(|_, path: String| Ok(expand_home(&path).exists()))?,
        )?;
        w.set(
            "mkdir",
            lua.create_function(|_, path: String| {
                std::fs::create_dir_all(expand_home(&path)).map_err(lua_err)
            })?,
        )?;

        // wlens.http_headers("https://media.example:9999/", { ApiKey = key })
        // Sent with every request (API, images, thumbnails) to URLs with that prefix.
        w.set(
            "http_headers",
            lua.create_function(|_, (prefix, headers): (String, Option<Table>)| {
                let mut list = Vec::new();
                if let Some(h) = headers {
                    for pair in h.pairs::<String, String>() {
                        let (k, v) = pair?;
                        // Say which header is bad now, rather than every
                        // request failing with "failed to parse header value".
                        if ureq::http::HeaderName::from_bytes(k.as_bytes()).is_err() {
                            return Err(lua_err(format!("http_headers: '{k}' isn't a valid header name")));
                        }
                        if let Some(c) = v.chars().find(|&c| c.is_control() && c != '\t' || !c.is_ascii()) {
                            return Err(lua_err(format!(
                                "http_headers: the {k} value contains {c:?}, which HTTP doesn't allow"
                            )));
                        }
                        list.push((k, v));
                    }
                }
                crate::io::remote::set_headers(&prefix, list).map_err(lua_err)?;
                Ok(())
            })?,
        )?;

        // wlens.json.decode(str) / wlens.json.encode(value)
        let json = lua.create_table()?;
        json.set(
            "decode",
            lua.create_function(|lua, s: String| {
                let v: serde_json::Value = serde_json::from_str(&s).map_err(lua_err)?;
                // JSON null -> nil, so `x.title or "untitled"` works as expected
                // (a null sentinel would be truthy).
                let opts = mlua::serde::SerializeOptions::new()
                    .serialize_none_to_null(false)
                    .serialize_unit_to_null(false);
                lua.to_value_with(&v, opts)
            })?,
        )?;
        json.set(
            "encode",
            lua.create_function(|lua, v: LuaValue| {
                let j: serde_json::Value = lua.from_value(v)?;
                serde_json::to_string(&j).map_err(lua_err)
            })?,
        )?;
        w.set("json", json)?;

        // wlens.mode() -> "image" | "gallery" | "video"
        let sh = shared.clone();
        w.set(
            "mode",
            lua.create_function(move |_, ()| {
                Ok(sh.borrow().info.as_ref().map_or("image", |i| i.mode))
            })?,
        )?;

        // wlens.unbind_all() — start from a blank slate instead of the defaults.
        let sh = shared.clone();
        w.set(
            "unbind_all",
            lua.create_function(move |_, ()| {
                let mut sh = sh.borrow_mut();
                sh.bindings.clear();
                sh.binding_revision += 1;
                Ok(())
            })?,
        )?;

        // wlens.on(event, fn)
        let sh = shared.clone();
        w.set(
            "on",
            lua.create_function(move |_, (event, f): (String, Function)| {
                sh.borrow_mut().hooks.entry(event).or_default().push(f);
                Ok(())
            })?,
        )?;

        // wlens.off(event, fn) — remove a hook added with wlens.on.
        let sh = shared.clone();
        w.set(
            "off",
            lua.create_function(move |_, (event, f): (String, Function)| {
                if let Some(list) = sh.borrow_mut().hooks.get_mut(&event) {
                    list.retain(|g| g != &f);
                }
                Ok(())
            })?,
        )?;

        // wlens.exec("zoom 200%") — validated now, run after the callback returns.
        let sh = shared.clone();
        w.set(
            "exec",
            lua.create_function(move |_, cmd: String| {
                let cmd: Command = cmd.parse().map_err(lua_err)?;
                sh.borrow_mut().queue.push(cmd);
                Ok(())
            })?,
        )?;

        // wlens.store.get(key) / wlens.store.set(key, value): this script's own
        // remembered values (strings, numbers, booleans, tables), kept between runs.
        let store = lua.create_table()?;
        let sh = shared.clone();
        store.set(
            "get",
            lua.create_function(move |lua, key: String| {
                let id = sh.borrow().script_id.clone();
                match crate::io::state::section(&id, "store").get(&key) {
                    Some(v) => lua.to_value_with(
                        v,
                        mlua::serde::SerializeOptions::new()
                            .serialize_none_to_null(false)
                            .serialize_unit_to_null(false),
                    ),
                    None => Ok(LuaValue::Nil),
                }
            })?,
        )?;
        let sh = shared.clone();
        store.set(
            "set",
            lua.create_function(move |lua, (key, v): (String, LuaValue)| {
                let id = sh.borrow().script_id.clone();
                let value = if v.is_nil() {
                    None
                } else {
                    Some(lua.from_value::<serde_json::Value>(v)?)
                };
                crate::io::state::put(&id, "store", &key, value);
                Ok(())
            })?,
        )?;
        w.set("store", store)?;

        // wlens.set(key, value) / wlens.get(key)
        let sh = shared.clone();
        let set = lua.create_function(move |_, (key, v): (String, LuaValue)| {
            let v = to_opt_string(&v)?;
            let mut sh = sh.borrow_mut();
            sh.settings_dirty = true;
            sh.settings.set(&key, &v).map_err(lua_err)?;
            // Changed while running (a menu, a key): remember it.
            if sh.started {
                sh.changed_options.push(key);
            }
            Ok(())
        })?;
        w.set("set", set.clone())?;
        let sh = shared.clone();
        let get = lua.create_function(move |lua, key: String| -> mlua::Result<LuaValue> {
            Ok(match sh.borrow().settings.get(&key) {
                None => LuaValue::Nil,
                Some(Value::Str(s)) => LuaValue::String(lua.create_string(&s)?),
                Some(Value::Num(n)) => LuaValue::Number(n),
                Some(Value::Bool(b)) => LuaValue::Boolean(b),
            })
        })?;
        w.set("get", get.clone())?;

        // wlens.opt.fit = "cover"  (sugar over set/get)
        let opt = lua.create_table()?;
        let meta = lua.create_table()?;
        meta.set(
            "__index",
            lua.create_function(move |_, (_t, k): (Table, String)| get.call::<LuaValue>(k))?,
        )?;
        meta.set(
            "__newindex",
            lua.create_function(move |_, (_t, k, v): (Table, String, LuaValue)| {
                set.call::<()>((k, v))
            })?,
        )?;
        opt.set_metatable(Some(meta))?;
        w.set("opt", opt)?;

        // wlens.theme { window = { background = "#1e1e2e" }, image = { radius = 12 } }
        // Nested tables flatten to dotted keys, so this is the same as
        // `set window.background #1e1e2e` and `wlens.opt["image.radius"] = 12`.
        let sh = shared.clone();
        w.set(
            "theme",
            lua.create_function(move |_, t: Table| {
                let mut flat = Vec::new();
                flatten("", LuaValue::Table(t), &mut flat)?;
                let mut sh = sh.borrow_mut();
                sh.settings_dirty = true;
                for (k, v) in flat {
                    sh.settings.set(&k, &v).map_err(lua_err)?;
                }
                Ok(())
            })?,
        )?;

        // wlens.reset_theme() — back to built-in defaults and no UI. Presets
        // call this first so switching between them doesn't leave residue.
        let sh = shared.clone();
        w.set(
            "reset_theme",
            lua.create_function(move |_, ()| {
                let mut sh = sh.borrow_mut();
                sh.settings.theme = Default::default();
                sh.pending_ui = Some(Ui::default());
                Ok(())
            })?,
        )?;

        // wlens.ui { {anchor = "bottom", ...}, ... } — replaces the overlay UI.
        let sh = shared.clone();
        w.set(
            "ui",
            lua.create_function(move |_, t: Table| {
                let ui = Ui::parse(t)?;
                sh.borrow_mut().pending_ui = Some(ui);
                Ok(())
            })?,
        )?;

        // wlens.input(id) -> text of a text input; wlens.input(id, text) sets it.
        let sh = shared.clone();
        w.set(
            "input",
            lua.create_function(move |_, (id, value): (String, Option<String>)| {
                let mut s = sh.borrow_mut();
                if let Some(v) = value {
                    s.inputs.insert(id.clone(), v);
                    s.ui_dirty = true;
                }
                Ok(s.inputs.get(&id).cloned().unwrap_or_default())
            })?,
        )?;

        // wlens.title(str)
        let sh = shared.clone();
        w.set(
            "title",
            lua.create_function(move |_, t: String| {
                sh.borrow_mut().title = Some(t);
                Ok(())
            })?,
        )?;

        // wlens.current() -> { path, name, dir, index, count, width, height, zoom } | nil
        let sh = shared.clone();
        w.set(
            "current",
            lua.create_function(move |lua, ()| -> mlua::Result<LuaValue> {
                let Some(info) = sh.borrow().info.clone() else {
                    return Ok(LuaValue::Nil);
                };
                Ok(LuaValue::Table(info_table(lua, &info)?))
            })?,
        )?;

        // wlens.spawn(cmd) — run via sh -c without blocking. The current file
        // is passed as $WLENS_FILE so scripts never need to shell-quote paths.
        let sh = shared.clone();
        w.set(
            "spawn",
            lua.create_function(move |_, cmd: String| {
                let mut c = std::process::Command::new("sh");
                c.arg("-c").arg(&cmd);
                if let Some(info) = &sh.borrow().info {
                    // For URLs, point at the downloaded copy when we have one.
                    let file = if info.remote {
                        c.env("WLENS_URL", &info.path);
                        crate::io::remote::cached_path(&info.path)
                            .filter(|p| p.exists())
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| info.path.clone())
                    } else {
                        info.path.clone()
                    };
                    c.env("WLENS_FILE", file)
                        .env("WLENS_INDEX", (info.index + 1).to_string())
                        .env("WLENS_COUNT", info.count.to_string());
                }
                c.spawn().map_err(lua_err)?;
                Ok(())
            })?,
        )?;

        // wlens.features: what this build can do.
        let features = lua.create_table()?;
        features.set("video", cfg!(feature = "video"))?;
        w.set("features", features)?;

        lua.globals().set("wlens", w)?;

        // Presets via package.preload, user modules from the config dir.
        let package: Table = lua.globals().get("package")?;
        let preload: Table = package.get("preload")?;
        for (name, src) in PRESETS {
            let f = lua
                .load(*src)
                .set_name(format!("=themes/{name}.lua"))
                .into_function()?;
            preload.set(format!("wlens.themes.{name}"), f)?;
        }
        for (name, src) in PALETTES {
            let f = lua
                .load(*src)
                .set_name(format!("=palettes/{name}.lua"))
                .into_function()?;
            preload.set(format!("wlens.palettes.{name}"), f)?;
        }
        if let Some(dir) = dirs::config_dir().map(|d| d.join("wlens")) {
            let path: String = package.get("path")?;
            let d = dir.display();
            // ~/.config/wlens first, then themes/scripts a package installed.
            package.set(
                "path",
                format!("{d}/?.lua;{d}/?/init.lua;/usr/share/wlens/?.lua;{path}"),
            )?;
        }
        lua.load(PRELUDE).set_name("=prelude").exec()?;
        let widgets: Table = lua.load(WIDGETS).set_name("=widgets.lua").eval()?;
        lua.globals()
            .get::<Table>("wlens")?
            .set("widgets", widgets)?;

        Ok(Self { lua, shared })
    }
}

/// `~/x` -> `$HOME/x`.
pub(super) fn expand_home(p: &str) -> std::path::PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| p.into()),
        None if p == "~" => dirs::home_dir().unwrap_or_else(|| p.into()),
        None => p.into(),
    }
}
