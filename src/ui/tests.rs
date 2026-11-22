//! UI parsing, layout and hit-testing tests.

use super::*;

#[test]
fn templates() {
    let f = |k: &str| match k {
        "name" => Some("a.png".to_string()),
        "index" => Some("3".to_string()),
        _ => None,
    };
    assert_eq!(expand("{name} [{index}]", &f), "a.png [3]");
    assert_eq!(expand("{nope} {{x}", &f), "{nope} {x}");
    assert_eq!(expand("open {", &f), "open {");
}

pub struct Fixed;
impl Measure for Fixed {
    fn text_size(&mut self, text: &str, _: &str, size: f32) -> (f32, f32) {
        (text.len() as f32 * size / 2.0, size)
    }
}

#[test]
fn bar_layout() {
    let lua = mlua::Lua::new();
    let tree: Table = lua
        .load(r#"return {
            { id = "bar", anchor = "bottom", width = "100%", height = 30, padding = {0, 10}, reserve = true,
              { text = "left", grow = true, text_align = "left" },
              { text = "right" } },
            { anchor = "top-right", x = 8, y = 8, text = "x", padding = 4 },
        }"#)
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    let theme = Theme {
        font_size: 10.0,
        ..Theme::default()
    };
    ui.layout(Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, &theme, &mut Fixed);
    let bar = ui.nodes[ui.roots[0]].rect;
    assert_eq!(bar, Rect::new(0.0, 270.0, 400.0, 30.0));
    let right = ui.nodes[ui.nodes[ui.roots[0]].children[1]].rect;
    assert_eq!(
        right.x + right.w,
        390.0,
        "right label hugs the padding edge"
    );
    let left = ui.nodes[ui.nodes[ui.roots[0]].children[0]].rect;
    assert_eq!(left.w, 380.0 - right.w, "grow takes the rest");
    let close = ui.nodes[ui.roots[1]].rect;
    assert_eq!((close.x, close.y, close.w), (400.0 - 8.0 - 13.0, 8.0, 13.0));
    assert_eq!(
        ui.reserved(Rect::new(0.0, 0.0, 400.0, 300.0), false),
        [0.0, 0.0, 30.0, 0.0]
    );
}

#[test]
fn back_button_keeps_its_slot_across_image_and_grid_views() {
    let script = crate::lua::Script::new().unwrap();
    let tree = script
        .lua
        .load("local w = wlens.widgets; return { w.bar { left = { w.menu_button(), w.navigation_button(), w.refresh_button() } } }")
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    let index = |id: &str| {
        ui.nodes
            .iter()
            .position(|n| n.id.as_deref() == Some(id))
            .unwrap()
    };
    let back = index("back_button");
    let refresh = index("refresh_button");
    let theme = Theme::default();
    let mut next_button = None;
    let mut back_rect = None;
    for (mode, depth, available) in [
        ("image", 1, true),
        ("gallery", 1, true),
        ("gallery", 0, false),
        ("image", 0, true),
    ] {
        script.shared.borrow_mut().depth = depth;
        let img = script.lua.create_table().unwrap();
        img.set("mode", mode).unwrap();
        img.set("count", 2).unwrap();
        ui.refresh(&str::to_owned, &LuaValue::Table(img));
        ui.animate(0.016, &theme, None, false);
        ui.layout(Rect::new(0.0, 0.0, 360.0, 200.0), 1.0, &theme, &mut Fixed);
        ui.draw(
            1.0,
            &theme,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut |_| None,
        );
        let rect = ui.nodes[refresh].rect;
        assert_eq!(rect, *next_button.get_or_insert(rect));
        if available {
            let rect = ui.nodes[back].rect;
            assert!(rect.w > 0.0 && rect.h > 0.0);
            assert_eq!(rect, *back_rect.get_or_insert(rect));
        }
        assert_eq!(
            ui.hit(back_rect.unwrap().center()).clickable,
            available.then_some(back)
        );
    }
}

fn build(src: &str) -> Ui {
    let lua = mlua::Lua::new();
    let mut ui = Ui::parse(lua.load(src).eval().unwrap()).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    let theme = Theme {
        font_size: 10.0,
        ..Theme::default()
    };
    ui.layout(Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, &theme, &mut Fixed);
    for n in &mut ui.nodes {
        n.opacity = 1.0;
    }
    ui
}

#[test]
fn justify_and_three_section_bar() {
    let ui = build(
        r#"return {
        { anchor = "bottom", width = "100%", height = 20,
          { grow = true, justify = "start", { text = "L" } },
          { text = "CC" },
          { grow = true, justify = "end", { text = "R" } } } }"#,
    );
    let bar = &ui.nodes[ui.roots[0]];
    let [l, c, r] = [bar.children[0], bar.children[1], bar.children[2]];
    let label = |i: usize| ui.nodes[ui.nodes[i].children[0]].rect;
    assert_eq!(label(l).x, 0.0);
    assert_eq!(
        ui.nodes[c].rect.center().0,
        200.0,
        "centre group is truly centred"
    );
    assert_eq!(label(r).x + label(r).w, 400.0);
}

#[test]
fn panels_swallow_clicks_scrims_catch_the_rest() {
    let ui = build(
        r##"return {
        { width = "100%", height = "100%", on_click = "ui hide m", justify = "center", align = "center",
          direction = "column",
          { width = 100, height = 100, background = "#222",
            { text = "item", on_click = "next" } } } }"##,
    );
    let scrim = ui.roots[0];
    let panel = ui.nodes[scrim].children[0];
    let item = ui.nodes[panel].children[0];
    assert_eq!(ui.hit((5.0, 5.0)).clickable, Some(scrim));
    let pr = ui.nodes[panel].rect;
    let empty_panel_spot = (pr.x + 2.0, pr.y + pr.h - 2.0);
    let h = ui.hit(empty_panel_spot);
    assert!(h.over_ui && h.clickable.is_none(), "panel swallows");
    assert_eq!(ui.hit(ui.nodes[item].rect.center()).clickable, Some(item));
}

#[test]
fn clicks_on_a_picture_reach_its_clickable_frame() {
    let ui = build(
        r##"return {
        { width = "100%", height = "100%", on_click = "ui hide m", justify = "center", align = "center",
          { padding = 2, on_click = "goto 3",
            { width = 50, height = 30, background = "#111", image = "x.png" } },
          { width = 60, height = 60, background = "#222",
            { width = 40, height = 40, background = "#333", image = "y.png" } } } }"##,
    );
    let scrim = ui.roots[0];
    let frame = ui.nodes[scrim].children[0];
    let pic = ui.nodes[frame].children[0];
    let panel = ui.nodes[scrim].children[1];
    let panel_pic = ui.nodes[panel].children[0];
    assert_eq!(ui.hit(ui.nodes[pic].rect.center()).clickable, Some(frame));
    let h = ui.hit(ui.nodes[panel_pic].rect.center());
    assert!(
        h.over_ui && h.clickable.is_none(),
        "stops at the panel, never the scrim"
    );
}

#[test]
fn follow_hover_lights_child_from_parent() {
    let mut ui = build(
        r##"return {
        { width = 100, height = 300, on_click = "prev",
          { width = 20, height = 20, follow_hover = true, opacity = 0, hover = { opacity = 1 } } } }"##,
    );
    let zone = ui.roots[0];
    let chevron = ui.nodes[zone].children[0];
    let theme = Theme {
        anim_ui: 0.0,
        ..Theme::default()
    };
    // cursor in the zone but far from the chevron
    ui.animate(0.016, &theme, Some((50.0, 290.0)), true);
    assert_eq!(ui.nodes[chevron].hover_t, 1.0);
    ui.animate(0.016, &theme, Some((250.0, 290.0)), true);
    assert_eq!(ui.nodes[chevron].hover_t, 0.0);
    assert!(ui.hit((50.0, 290.0)).pointer);
}

#[test]
fn children_keep_index_order_and_view_gets_a_rect() {
    let lua = mlua::Lua::new();
    // Built like w.layout does: named keys first, children appended later
    // (they can land in the table's hash part, where pairs() order is arbitrary).
    let tree: Table = lua
        .load(
            r#"
            local row = { dir = "row", width = "100%", height = "100%", align = "start", gap = 0 }
            for k = 1, 8 do row["k" .. k] = nil end
            row[#row + 1] = { width = 50, height = 10, id = "left" }
            row[#row + 1] = { view = "main", grow = true, height = "100%" }
            row[#row + 1] = { width = 70, height = 10, id = "right" }
            return { row }
        "#,
        )
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    ui.layout(
        Rect::new(0.0, 0.0, 400.0, 300.0),
        1.0,
        &Theme::default(),
        &mut Fixed,
    );
    let x = |id: &str| {
        ui.nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(id))
            .unwrap()
            .rect
            .x
    };
    assert_eq!(x("left"), 0.0);
    assert_eq!(x("right"), 330.0);
    assert_eq!(
        ui.view_rect("main"),
        Some(Rect::new(50.0, 0.0, 280.0, 300.0))
    );
}

#[test]
fn dynamic_children_rebuild_and_compact() {
    let lua = mlua::Lua::new();
    let tree: Table = lua
        .load(
            r#"return {
            { id = "before", text = "a" },
            { id = "list", dir = "column",
              children = function(img)
                local out = {}
                for i = 1, img.n do out[i] = { id = "row" .. i, text = "row " .. i } end
                return out
              end },
            { id = "after", text = "z" },
        }"#,
        )
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    let info = |n: i64| {
        let t = lua.create_table().unwrap();
        t.set("n", n).unwrap();
        LuaValue::Table(t)
    };
    let ids = |ui: &Ui| {
        ui.nodes
            .iter()
            .filter_map(|n| n.id.clone())
            .collect::<Vec<_>>()
    };
    assert!(ui.update_dynamic(&info(3), "k1"));
    assert_eq!(
        ids(&ui),
        ["before", "list", "row1", "row2", "row3", "after"]
    );
    // Same key: nothing rebuilt.
    assert!(!ui.update_dynamic(&info(1), "k1"));
    assert!(ui.update_dynamic(&info(1), "k2"));
    assert_eq!(ids(&ui), ["before", "list", "row1", "after"]);
    // Parents come before children and links are consistent after compaction.
    for (i, n) in ui.nodes.iter().enumerate() {
        for &c in &n.children {
            assert!(c > i);
            assert_eq!(ui.nodes[c].parent, Some(i));
        }
    }
    assert_eq!(ui.roots.len(), 3);
}

#[test]
fn scroll_and_wrap() {
    let lua = mlua::Lua::new();
    let tree: Table = lua
        .load(
            r#"
            local list = { id = "list", dir = "column", width = 100, height = 100, scroll = true }
            for i = 1, 10 do list[#list + 1] = { id = "r" .. i, width = 100, height = 30 } end
            local chips = { id = "chips", dir = "row", wrap = true, width = 100, gap = 10 }
            for i = 1, 5 do chips[#chips + 1] = { width = 40, height = 20 } end
            return { { dir = "column", list, chips } }
        "#,
        )
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    let win = Rect::new(0.0, 0.0, 400.0, 400.0);
    ui.layout(win, 1.0, &Theme::default(), &mut Fixed);
    let node = |ui: &Ui, id: &str| {
        ui.nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(id))
            .map(|n| (n.rect, n.clip))
            .unwrap()
    };
    // 10 rows of 30 in a 100-tall box: content 300, rows clipped to the box.
    let (r3, clip) = node(&ui, "r3");
    assert_eq!(r3.y, 60.0);
    assert_eq!(clip, Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    // Two chips (40 + 10 + 40) per 100px line: 5 chips -> 3 lines of 20 + 2 gaps.
    assert_eq!(node(&ui, "chips").0.h, 80.0);
    // Wheel down 50px over the list moves its rows up; clamped at the end.
    for n in &mut ui.nodes {
        n.opacity = 1.0;
    }
    assert!(ui.scroll_at((50.0, 50.0), -50.0));
    ui.layout(win, 1.0, &Theme::default(), &mut Fixed);
    assert_eq!(node(&ui, "r3").0.y, 10.0);
    assert!(ui.scroll_at((50.0, 50.0), -1000.0));
    ui.layout(win, 1.0, &Theme::default(), &mut Fixed);
    assert_eq!(node(&ui, "r10").0.y, 70.0); // last row's bottom at the box's bottom
}

#[test]
fn overflowing_row_shrinks_text_instead_of_overlapping() {
    let lua = mlua::Lua::new();
    // 300px row: a long name (shrinkable) + two fixed buttons that need 200px.
    let tree: Table = lua
        .load(
            r#"return {
            { dir = "row", width = 300, height = 20, gap = 0,
              { id = "name", text = string.rep("x", 60), max_width = 400 },
              { id = "a", width = 100, height = 20 },
              { id = "b", width = 100, height = 20 } },
        }"#,
        )
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    let theme = Theme {
        font_size: 10.0,
        ..Theme::default()
    };
    ui.layout(Rect::new(0.0, 0.0, 400.0, 100.0), 1.0, &theme, &mut Fixed);
    let get = |id: &str| {
        ui.nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(id))
            .unwrap()
    };
    let (name, a, b) = (get("name"), get("a"), get("b"));
    // The name gave up exactly the overflow; nothing overlaps or spills out.
    assert!(
        name.rect.x + name.rect.w <= a.rect.x + 0.5,
        "name {:?} overlaps a {:?}",
        name.rect,
        a.rect
    );
    assert!(b.rect.x + b.rect.w <= 300.5);
    assert!(name.display_text.contains('…'));
    assert!(
        !name.text_value.contains('…'),
        "the source text stays whole"
    );
}

#[test]
fn top_layer_wins_over_later_base_nodes() {
    let ui = build(
        r##"return {
        { id = "overlay", layer = "top", width = 100, height = 100, background = "#222", on_click = "next" },
        { id = "base", width = 100, height = 100, background = "#444", on_click = "prev" }
    }"##,
    );
    let hit = ui.hit((10.0, 10.0)).clickable.unwrap();
    assert_eq!(ui.nodes[hit].id.as_deref(), Some("overlay"));
}

#[test]
fn input_visibility_includes_ancestors_and_removed_inputs() {
    let mut ui = build(
        r##"return {
        { id = "sidebar", width = 100, height = 100,
          { id = "search", input = { placeholder = "Search" } } }
    }"##,
    );
    assert!(ui.input_visible("search"));
    ui.set_forced("sidebar", "hide").unwrap();
    assert!(!ui.input_visible("search"));
    assert!(!ui.input_visible("missing"));
}

#[test]
fn floating_max_height_and_wrapping_scroll() {
    let lua = mlua::Lua::new();
    let tree: Table = lua
        .load(r#"
            local chips = { id = "chips", dir = "row", wrap = true, scroll = true,
                            width = 100, height = 50, gap = 10 }
            for i = 1, 6 do chips[#chips + 1] = { id = "c" .. i, width = 40, height = 20 } end
            return { { width = "100%", height = "100%",
                       { id = "float", anchor = "top-left", width = 50, height = 500, max_height = "50%" },
                       chips } }
        "#)
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    let win = Rect::new(0.0, 0.0, 400.0, 200.0);
    ui.layout(win, 1.0, &Theme::default(), &mut Fixed);
    let r = |ui: &Ui, id: &str| {
        ui.nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(id))
            .unwrap()
            .rect
    };
    assert_eq!(
        r(&ui, "float").h,
        100.0,
        "floating element capped at 50% of its parent"
    );
    // 6 chips, 2 per line -> 3 lines (20 + 10 + 20 + 10 + 20 = 80) in a 50px box.
    for n in &mut ui.nodes {
        n.opacity = 1.0;
    }
    let chips = r(&ui, "chips");
    let before = r(&ui, "c5").y;
    assert!(ui.scroll_at((chips.x + 5.0, chips.y + 5.0), -100.0));
    ui.layout(win, 1.0, &Theme::default(), &mut Fixed);
    assert_eq!(
        before - r(&ui, "c5").y,
        30.0,
        "scrolled to the end: 80 content - 50 box"
    );
}

#[test]
fn toggling_an_element_that_has_a_show_function() {
    let lua = mlua::Lua::new();
    lua.load("allowed = true").exec().unwrap();
    let tree: Table = lua
        .load(r#"return { { id = "panel", text = "x", show = function() return allowed end } }"#)
        .eval()
        .unwrap();
    let mut ui = Ui::parse(tree).unwrap();
    let refresh = |ui: &mut Ui| ui.refresh(&|t| t.to_string(), &LuaValue::Nil);
    refresh(&mut ui);
    assert!(ui.visible("panel"));
    ui.set_forced("panel", "toggle").unwrap();
    assert!(!ui.visible("panel"), "toggle hides it");
    ui.set_forced("panel", "toggle").unwrap();
    assert!(ui.visible("panel"), "and toggles it back");
    // Hidden by its function: toggling hides it for good (once the
    // function allows it again it stays hidden), rather than doing nothing.
    lua.load("allowed = false").exec().unwrap();
    refresh(&mut ui);
    assert!(!ui.visible("panel"));
    ui.set_forced("panel", "show").unwrap();
    assert!(!ui.visible("panel"), "ui show can't override the function");
    ui.set_forced("panel", "toggle").unwrap();
    lua.load("allowed = true").exec().unwrap();
    refresh(&mut ui);
    assert!(!ui.visible("panel"));
}

#[test]
fn shortening_text() {
    let mut m = Fixed; // 5 px per character at size 10
    assert_eq!(
        shorten(&mut m, "short.jpg", "", 10.0, 100.0, false),
        "short.jpg"
    );
    let s = shorten(&mut m, "a-very-long-name.jpg", "", 10.0, 60.0, false);
    assert!(
        s.contains('…') && s.ends_with(".jpg") && s.len() <= 14,
        "{s}"
    );
    // (Fixed measures bytes, so "…" counts as three characters.)
    assert_eq!(shorten(&mut m, "abcdefghij", "", 10.0, 30.0, true), "abc…");
}

#[test]
fn pass_through_elements_let_the_mouse_reach_what_is_below() {
    let ui = build(
        r##"return {
        { anchor = "top-left", width = 100, height = 100, background = "#00000080", pass_through = true,
          { width = 20, height = 20, on_click = "next" } } }"##,
    );
    let tint = ui.roots[0];
    let button = ui.nodes[tint].children[0];
    let h = ui.hit((80.0, 80.0));
    assert!(!h.over_ui, "the tint doesn't take the mouse");
    assert_eq!(
        ui.hit(ui.nodes[button].rect.center()).clickable,
        Some(button)
    );
}
