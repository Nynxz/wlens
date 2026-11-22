//! Lua host tests: presets, palettes, theme tables.

use super::*;

#[test]
fn presets_and_theme_tables_load() {
    let s = Script::new().unwrap();
    s.run_str("=default", DEFAULT_CONFIG).unwrap();
    for (name, _) in PRESETS {
        s.run_str("=t", &format!("wlens.use('{name}')"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            s.shared.borrow_mut().pending_ui.take().is_some(),
            "{name} sets a ui"
        );
    }
    s.run_str(
        "=t",
        r##"wlens.theme { image = { radius = 7, checker = { "#111", "#222" } } }"##,
    )
    .unwrap();
    let sh = s.shared.borrow();
    assert_eq!(sh.settings.theme.image_radius, 7.0);
    assert_eq!(sh.settings.theme.checker_b.to_string(), "#222222ff");
    drop(sh);
    assert!(
        s.run_str("=t", "wlens.theme { image = { radiu = 1 } }")
            .is_err()
    );

    // Every example palette theme, in each mode, must map cleanly on both layouts.
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/lua/palettes");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let p = path.display().to_string();
        s.run_str(
            "=t",
            &format!(
                r#"
                local pal = dofile({p:?})
                assert(type(pal) == "table" and (pal.dark or pal.light), "not a palette")
                for _, mode in ipairs {{ "dark", "light" }} do
                  if pal[mode] then
                    for _, theme in ipairs {{ "simple", "lightbox" }} do
                      wlens.use(theme, {{ palette = {p:?} .. ":" .. mode, fullscreen = false }})
                    end
                  end
                end
                "#
            ),
        )
        .unwrap_or_else(|e| panic!("{p}: {e}"));
        checked += 1;
    }
    assert!(checked >= 3, "expected example palettes in {dir}");
    // A palette given inline, with either key style.
    s.run_str("=t", r##"wlens.use("simple", { palette = { background = "#101418", ["--primary"] = "oklch(0.7 0.15 250)" } })"##)
        .unwrap();
    assert_eq!(
        s.shared
            .borrow()
            .settings
            .theme
            .window_background
            .to_string(),
        "#101418ff"
    );
}

/// The example adapters run against controlled asynchronous callbacks, without
/// contacting servers, loading user settings, or constructing a window.
fn adapter_lua() -> Lua {
    let lua = Lua::new();
    lua.load(
        r#"
        requests, downloads, timers, hooks, bindings = {}, {}, {}, {}, {}
        depth, more, shown, item_sets, notices = 0, false, {}, 0, {}
        local now = 100
        os.time = function() return now end
        advance = function(s) now = now + s end
        os.getenv = function() return nil end
        wlens = {
          features = {}, opt = {},
          store = { get = function() end, set = function() end },
          json = { decode = function(body) return body end },
          fetch = function(url, opts, cb)
            requests[#requests+1] = { url=url, cb=cb or opts }
          end,
          download = function(url, path, cb) downloads[#downloads+1] = cb end,
          after = function(_, cb) timers[#timers+1] = cb end,
          on = function(event, cb) hooks[event] = cb end,
          bind = function(key, cb) bindings[key] = cb end,
          depth = function() return depth end,
          set_more = function(v) more = v end,
          set_items = function(v) shown = v; item_sets = item_sets + 1 end,
          add_items = function(v) for _, i in ipairs(v) do shown[#shown+1] = i end end,
          push_items = function(v) depth = depth+1; shown = v end,
          pop_items = function() depth = depth-1 end,
          current = function() return nil end,
          toast = function(message) notices[#notices+1] = message end,
          exists = function() return false end,
        }
        setmetatable(wlens, { __index = function() return function() end end })
        wlens.widgets = setmetatable({ icons = {}, style = {} }, {
          __index = function() return function(o) return type(o) == 'table' and o or {} end end
        })
    "#,
    )
    .exec()
    .unwrap();
    lua
}

#[test]
fn comfyui_refresh_discards_older_library_responses() {
    let lua = adapter_lua();
    let module: Table = lua
        .load(include_str!("../../examples/scripts/comfyui.lua"))
        .call("comfyui")
        .unwrap();
    lua.globals().set("comfy", module).unwrap();
    lua.load(
        r#"
        comfy.setup { url='http://localhost', group='none', source='assets' }
        comfy.open()
        comfy.open()
        requests[2].cb({assets={{id='new', mime_type='image/png'}}, total=1, has_more=false})
        assert(#shown == 1 and shown[1].src:find('/new/'))
        local sets = item_sets
        requests[1].cb({assets={{id='old', mime_type='image/png'}}, total=1, has_more=false})
        assert(item_sets == sets and shown[1].src:find('/new/'))
    "#,
    )
    .exec()
    .unwrap();
}

#[test]
fn commands_are_functions() {
    use crate::model::command::{Command, Zoom};
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r#"wlens.next(4); wlens.go(12.0); wlens.zoom("in"); wlens.video("seek", "+10")
           wlens.toggle("help"); wlens.prev()"#,
    )
    .unwrap();
    let q = std::mem::take(&mut s.shared.borrow_mut().queue);
    assert_eq!(
        q,
        vec![
            Command::Next(4),
            Command::Goto(11),
            Command::Zoom(Zoom::In),
            Command::Video("seek".into(), Some("+10".into())),
            Command::Ui("toggle".into(), "help".into()),
            Command::Next(-1),
        ]
    );
    let err = s.run_str("=t", r#"wlens.gallery("sideways")"#).unwrap_err();
    assert!(err.to_string().contains("unknown action"), "{err}");
}

#[test]
fn themes_build_on_each_other_under_theme_flag() {
    // --theme replaces only the outermost wlens.use: presets that start from
    // `minimal` mustn't turn into themselves (and recurse) under the flag.
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r#"wlens.args = { theme = "lightbox" }; wlens.use("simple")"#,
    )
    .unwrap();
    s.run_str(
        "=t",
        r#"wlens.args = { theme = "simple" }; wlens.use("minimal")"#,
    )
    .unwrap();
    assert!(s.shared.borrow_mut().pending_ui.take().is_some());
}

#[test]
fn buttons_take_their_parts_in_order_or_by_name() {
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r##"
        local w = wlens.widgets
        local a = w.button("x", "next", "Next", { color = "#ff0000" })
        local b = w.button { icon = "x", action = "next", tooltip = "Next", color = "#ff0000" }
        for _, el in ipairs { a, b } do
          assert(el.text == "x" and el.tooltip == "Next" and el.color == "#ff0000", "fields")
          assert(type(el.on_click) == "function", "action")
          assert(el.icon == nil and el.action == nil, "no stray keys")
        end
        local act = w.action { label = "Go", action = "next", primary = true }
        assert(act.text == "Go" and act.on_click == "next" and act.primary == nil)
        "##,
    )
    .unwrap();
}

#[test]
fn video_controls_show_their_state() {
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r##"
        local w = wlens.widgets
        local play = w.play_button()
        assert(play.text({ paused = true }) == w.icons.play, "paused shows play")
        assert(play.text({ paused = false }) == w.icons.pause, "playing shows pause")
        local vol = w.volume { width = 40 }
        local speaker, bar = vol[1], vol[2]
        assert(speaker.text({ muted = true, volume = 50 }) == w.icons.muted, "muted")
        assert(speaker.text({ muted = false, volume = 0 }) == w.icons.muted, "silent")
        assert(speaker.text({ muted = false, volume = 50 }) == w.icons.volume, "sound")
        assert(bar.width == 40 and bar.slider, "bar")
        -- the presets build with them
        wlens.use("simple"); wlens.use("lightbox")
        "##,
    )
    .unwrap();
}

#[test]
fn things_in_the_view_float_over_it() {
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r##"
        local w = wlens.widgets
        local root = w.layout { view = w.list(w.nav_zones(), w.menu(), w.help(), w.toast()) }
        local view
        for _, row in ipairs(root) do
          for _, it in ipairs(row) do if it.view == "main" then view = it end end
        end
        assert(view and #view == 4, "view items")
        for i = 1, 3 do assert(view[i].anchor == "top-left", "item " .. i .. " floats") end
        assert(view[4].anchor == "top", "an anchor of its own is kept")
        "##,
    )
    .unwrap();
}

#[test]
fn every_layout_region_is_a_panel() {
    let s = Script::new().unwrap();
    s.run_str(
        "=t",
        r##"
        local w = wlens.widgets
        local root = w.layout { top = { id = "t" }, bottom = { id = "b" },
                                left = { id = "l" }, right = { id = "r" } }
        local panels = 0
        local function walk(e)
          if e.id == "panels" then panels = panels + 1 end
          for _, c in ipairs(e) do if type(c) == "table" then walk(c) end end
        end
        walk(root)
        assert(panels == 4, "top, bottom, left and right: " .. panels)
        "##,
    )
    .unwrap();
}

#[test]
fn folder_browser_exposes_archives_as_containers() {
    let s = Script::new().unwrap();
    s.lua
        .globals()
        .set(
            "fixture_dir",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"),
        )
        .unwrap();
    s.run_str("=browse", "wlens.widgets.browse(fixture_dir)")
        .unwrap();
    let sh = s.shared.borrow();
    let ItemsOp::Set(items, _) = &sh.pending_items[0] else {
        panic!("expected folder contents")
    };
    assert_eq!(items.len(), 4);
    assert!(
        items
            .iter()
            .all(|item| item.container && item.icon.is_some())
    );
}
