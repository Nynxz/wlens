//! Lua scripting host. Lua lives on the main thread only.
//!
//! Lua never touches app state directly: API functions mutate `Shared`
//! (settings, bindings, hooks) or push `Command`s onto a queue the app
//! drains after every Lua call. That keeps borrows simple and means
//! anything Lua can do, a keybind or IPC client can do too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use mlua::{Function, Lua, LuaSerdeExt, Table, Value as LuaValue};

use crate::model::command::Command;
use crate::model::item::{Item, Source};
use crate::model::keys::Chord;
use crate::model::settings::{Settings, Value};
use crate::ui::Ui;

mod api;
mod info;
#[cfg(test)]
mod tests;

pub use info::ImageInfo;

pub const DEFAULT_CONFIG: &str = include_str!("default.lua");

/// Built-in theme presets, loadable with `wlens.use(name)` or `require`.
/// The Lua widget library, exposed as `wlens.widgets`.
const WIDGETS: &str = include_str!("widgets.lua");

const PRESETS: &[(&str, &str)] = &[
    ("minimal", include_str!("themes/minimal.lua")),
    ("simple", include_str!("themes/simple.lua")),
    ("lightbox", include_str!("themes/lightbox.lua")),
];

/// Built-in palettes. A file of the same name in ~/.config/wlens/themes wins.
const PALETTES: &[(&str, &str)] = &[
    ("gruvbox", include_str!("palettes/gruvbox.lua")),
    ("tokyo-night", include_str!("palettes/tokyo-night.lua")),
    (
        "strawberry-milk",
        include_str!("palettes/strawberry-milk.lua"),
    ),
];

/// Lua-side helpers that are simpler to write in Lua itself.
const PRELUDE: &str = r##"
-- wlens.use("lightbox", { accent = "#cba6f7" })
-- Looks for a built-in preset first, then ~/.config/wlens/themes/<name>.lua.
-- `wlens --theme NAME` (or $WLENS_THEME) overrides every wlens.use() call,
-- so the default config, your init.lua and example scripts all honour it.
-- A theme may build on another (`wlens.use("minimal", o)` inside it): only
-- the outermost call is replaced by --theme, and the inner one's bindings
-- belong to the outer theme.
local use_depth = 0
function wlens.use(name, opts)
  if use_depth == 0 then name = (wlens.args and wlens.args.theme) or name end
  opts = opts or {}
  -- `--palette NAME` colours whichever theme is used, unless it chose its own.
  if opts.palette == nil and wlens.args and wlens.args.palette then
    opts = wlens.extend(opts, { palette = wlens.args.palette })
  end
  local ok, preset = pcall(require, "wlens.themes." .. name)
  if not ok then
    local ok2, user = pcall(require, "themes." .. name)
    if not ok2 then ok2, user = pcall(require, "wlens.palettes." .. name) end
    if not ok2 then
      error("unknown theme '" .. name .. "' (built-in: minimal, simple, lightbox, gruvbox, tokyo-night, strawberry-milk; or put "
        .. "your own in ~/.config/wlens/themes/" .. name .. ".lua)", 2)
    end
    preset = user
  end
  -- A theme file may also return a table of colours (a palette theme):
  -- it's applied on its `base` preset (default "simple").
  if type(preset) == "table" then
    local base = preset.base or "simple"
    local o = wlens.extend(opts, { palette = opts.palette or preset })
    local ok3, base_fn = pcall(require, "wlens.themes." .. base)
    if not ok3 then error("palette theme '" .. name .. "': unknown base '" .. base .. "'", 2) end
    return wlens._theme_scope(function() return base_fn(o) end)
  end
  if type(preset) ~= "function" then
    error("theme '" .. name .. "' must return a function(opts) or a palette table", 2)
  end
  return wlens._theme_scope(function() return preset(opts or {}) end)
end

-- What a theme registers (key bindings, hooks) belongs to it: switching
-- theme removes the previous one's, so nothing leaks or doubles up.
do
  local owned = { binds = {}, hooks = {} }
  function wlens._theme_scope(apply)
    if use_depth > 0 then return apply() end
    for _, b in ipairs(owned.binds) do wlens.bind(b[1], nil, nil, b[2]) end
    for _, h in ipairs(owned.hooks) do wlens.off(h[1], h[2]) end
    owned = { binds = {}, hooks = {} }
    local bind, on = wlens.bind, wlens.on
    wlens.bind = function(key, action, desc, mode)
      if action ~= nil then table.insert(owned.binds, { key, mode }) end
      return bind(key, action, desc, mode)
    end
    wlens.on = function(event, f)
      table.insert(owned.hooks, { event, f })
      return on(event, f)
    end
    use_depth = use_depth + 1
    local ok, res = pcall(apply)
    use_depth = use_depth - 1
    wlens.bind, wlens.on = bind, on
    if not ok then error(res, 0) end
    return res
  end
end

-- Remember the last UI tree, so widgets added to a slot later
-- (wlens.slot) can rebuild it.
do
  local set_ui = wlens.ui
  function wlens.ui(spec)
    wlens._ui_spec = spec
    return set_ui(spec)
  end
end

-- Every command is also a function, so scripts call actions directly
-- instead of building strings:
--   wlens.next(4)   wlens.go(12)   wlens.zoom("in")   wlens.video("seek", "+10")
--   wlens.toggle("help")   wlens.gallery("open")
-- (Command strings stay handy for keybindings: wlens.bind("x", "rotate 90").)
do
  local function arg(v)
    if math.type(v) == "float" and v == math.floor(v) then return string.format("%d", v) end
    return tostring(v)
  end
  local function command(name)
    return function(...)
      local parts = { name }
      for i = 1, select("#", ...) do
        local v = select(i, ...)
        if v ~= nil then parts[#parts + 1] = arg(v) end
      end
      wlens.exec(table.concat(parts, " "))
    end
  end
  for _, name in ipairs({
    "next", "prev", "first", "last", "zoom", "fit", "pan", "rotate", "flip", "reset",
    "fullscreen", "reload", "quit", "copy", "gallery", "back", "forward", "open", "refresh",
    "animation", "video",
  }) do
    wlens[name] = command(name)
  end
  wlens.go = command("goto") -- `goto` is a Lua keyword
  for _, how in ipairs({ "show", "hide", "toggle" }) do
    wlens[how] = function(id) wlens.exec("ui " .. how .. " " .. id) end
  end
  function wlens.focus(id) wlens.exec("ui focus " .. id) end
  function wlens.blur(id) wlens.exec("ui blur " .. id) end
end

-- Shallow-merge tables left to right: wlens.extend(base_style, { text = "x" })
function wlens.extend(...)
  local out = {}
  for _, t in ipairs({ ... }) do
    for k, v in pairs(t) do out[k] = v end
  end
  return out
end
"##;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Command(Command),
    Lua(Function),
}

/// A change to the item list requested from Lua.
pub enum ItemsOp {
    /// Replace the list (start index).
    Set(Vec<Item>, Option<usize>),
    Add(Vec<Item>),
    /// Enter a new level (start index, title); `back` returns.
    Push(Vec<Item>, Option<usize>, Option<String>),
    /// Leave the current level (like `back`, but ordered with the other ops).
    Pop,
    /// `wlens.set_more`: the current list may continue.
    More(bool),
    Directory(std::path::PathBuf),
}

pub struct Binding {
    pub action: Action,
    /// The chord as the user wrote it (`C-n`), for help screens.
    pub key: String,
    pub desc: String,
    /// The command string, when bound to one (for matching menu hints).
    pub command: Option<String>,
    order: u64,
}

pub struct Timer {
    pub id: u64,
    pub at: std::time::Instant,
    pub f: Function,
}

#[derive(Default)]
pub struct Shared {
    pub settings: Settings,
    /// Keyed by (mode, chord); mode `None` = everywhere.
    pub bindings: HashMap<(Option<String>, Chord), Binding>,
    next_order: u64,
    pub timers: Vec<Timer>,
    next_timer: u64,
    /// UI element visibility by id, mirrored from the app for `wlens.visible`.
    pub visible: HashMap<String, bool>,
    /// Which script's remembered state to use (its config path, or "default").
    pub script_id: String,
    /// Startup is over: option changes from now on are the user's, and are remembered.
    pub started: bool,
    /// Options changed since the last save (keys).
    pub changed_options: Vec<String>,
    /// Window size in logical px, for `wlens.window()`.
    pub window: (f64, f64),
    /// The main view (image / grid area) in logical px: x, y, w, h (`wlens.view()`).
    pub view: (f64, f64, f64, f64),
    pub hooks: HashMap<String, Vec<Function>>,
    pub queue: Vec<Command>,
    pub title: Option<String>,
    pub info: Option<ImageInfo>,
    /// A UI tree set by `wlens.ui{}` that the app hasn't picked up yet.
    pub pending_ui: Option<Ui>,
    /// Script errors not yet shown (the app toasts them).
    pub errors: Vec<String>,
    /// Text input values by element id (`wlens.input`).
    pub inputs: HashMap<String, String>,
    /// Script code ran (a callback, timer, hook…) or an input changed: the
    /// app re-evaluates the UI, so scripts never have to ask for a refresh.
    pub ui_dirty: bool,
    /// Bumped each time script code runs; `children = function` elements
    /// rebuild when it changes (they may read any script state).
    pub generation: u64,
    /// The item list, mirrored from the app for `wlens.items()`.
    pub items: Vec<Item>,
    /// `wlens.set_items` / `add_items` requests: (items, append, start index).
    pub pending_items: Vec<ItemsOp>,
    /// How many levels deep we are (mirrored from the app for `wlens.depth()`).
    pub depth: usize,
    /// `wlens.fetch` requests waiting to be handed to the loader.
    pub fetch_queue: Vec<(u64, crate::io::remote::Request)>,
    pub fetch_callbacks: HashMap<u64, Function>,
    /// `wlens.download` requests not yet started: (id, url, destination).
    pub download_queue: Vec<(u64, String, std::path::PathBuf)>,
    /// (on_progress, on_done) per download id.
    pub download_callbacks: HashMap<u64, (Option<Function>, Option<Function>)>,
    next_download: u64,
    next_fetch: u64,
    /// Set when Lua changed settings/theme, so the app refreshes.
    pub settings_dirty: bool,
    /// `wlens.set_more(true)`: the current list may continue (infinite scroll).
    pub more: bool,
}

pub struct Script {
    pub lua: Lua,
    pub shared: Rc<RefCell<Shared>>,
}

fn lua_err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::RuntimeError(e.to_string())
}

/// Lua value -> option string (`true`, `1.5`, `"cover"`).
fn to_opt_string(v: &LuaValue) -> mlua::Result<String> {
    Ok(match v {
        LuaValue::String(s) => s.to_str()?.to_string(),
        LuaValue::Integer(i) => i.to_string(),
        LuaValue::Number(n) => n.to_string(),
        LuaValue::Boolean(b) => b.to_string(),
        other => {
            return Err(lua_err(format!(
                "can't use a {} as an option value",
                other.type_name()
            )));
        }
    })
}

impl Script {
    /// Run a config / script: a `.lua` file, or a folder holding `init.lua`
    /// (its other files are `require`-able by name).
    pub fn run_file(&self, path: &Path) -> mlua::Result<()> {
        let path = if path.is_dir() {
            path.join("init.lua")
        } else {
            path.to_path_buf()
        };
        let path = path.as_path();
        let src = std::fs::read_to_string(path)
            .map_err(|e| lua_err(format!("{}: {e}", path.display())))?;
        // `wlens.config_file` / `wlens.script_dir`: find files next to the script.
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let dir = abs
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let w = self.lua.globals().get::<Table>("wlens")?;
        w.set("config_file", abs.to_string_lossy().as_ref())?;
        w.set("script_dir", dir.as_str())?;
        // `require("x")` looks next to the script first.
        let package: Table = self.lua.globals().get("package")?;
        let old: String = package.get("path")?;
        let mine = format!("{dir}/?.lua;{dir}/?/init.lua;");
        if !old.starts_with(&mine) {
            package.set("path", format!("{mine}{old}"))?;
        }
        self.lua
            .load(src)
            .set_name(format!("@{}", path.display()))
            .exec()
    }

    /// `wlens.args`: command-line choices scripts can read (e.g. `theme`).
    pub fn set_args(&self, theme: Option<&str>, palette: Option<&str>) -> mlua::Result<()> {
        let args = self.lua.create_table()?;
        args.set("theme", theme)?;
        args.set("palette", palette)?;
        self.lua.globals().get::<Table>("wlens")?.set("args", args)
    }

    pub fn directory_items(&self, path: &std::path::Path) -> mlua::Result<Vec<Item>> {
        let list: Table = self
            .lua
            .load("return wlens.widgets.directory_items(...)")
            .call(path.to_string_lossy().as_ref())?;
        parse_items(&list)
    }

    pub fn run_str(&self, name: &str, src: &str) -> mlua::Result<()> {
        self.lua.load(src).set_name(name).exec()
    }

    /// A mode-specific binding wins over a global one.
    pub fn binding(&self, chord: &Chord, mode: &str) -> Option<Action> {
        let sh = self.shared.borrow();
        // video -> image -> global: a video still gets image bindings it doesn't override.
        let chain: &[&str] = if mode == "video" {
            &["video", "image"]
        } else {
            std::slice::from_ref(&mode)
        };
        chain
            .iter()
            .find_map(|m| sh.bindings.get(&(Some(m.to_string()), chord.clone())))
            .or_else(|| sh.bindings.get(&(None, chord.clone())))
            .map(|b| b.action.clone())
    }

    /// Deliver a `wlens.download` progress report or result.
    pub fn download_event(
        &self,
        id: u64,
        progress: Option<(u64, Option<u64>)>,
        done: Option<Result<u64, String>>,
    ) {
        let cbs = if done.is_some() {
            self.shared.borrow_mut().download_callbacks.remove(&id)
        } else {
            self.shared.borrow().download_callbacks.get(&id).cloned()
        };
        let Some((on_progress, on_done)) = cbs else {
            return;
        };
        let r = match (progress, done) {
            (Some((bytes, total)), _) => match on_progress {
                Some(f) => f.call::<()>((bytes, total)),
                None => Ok(()),
            },
            (_, Some(res)) => match on_done {
                Some(f) => match res {
                    Ok(bytes) => f.call::<()>((LuaValue::Nil, bytes)),
                    Err(e) => f.call::<()>((e, LuaValue::Nil)),
                },
                None => Ok(()),
            },
            _ => Ok(()),
        };
        self.touched();
        if let Err(e) = r {
            self.report(format!("wlens.download callback: {e}"));
        }
    }

    /// Deliver a `wlens.fetch` result to its callback.
    pub fn fetched(&self, id: u64, result: crate::io::remote::FetchResult) {
        let Some(f) = self.shared.borrow_mut().fetch_callbacks.remove(&id) else {
            return;
        };
        // fn(body, err, status, headers): headers is a table, names lowercased.
        let headers_table = |h: &[(String, String)]| -> mlua::Result<Table> {
            let t = self.lua.create_table()?;
            for (k, v) in h {
                // Repeated headers are joined: Set-Cookie one per line,
                // others with ", " (as HTTP allows).
                let joined = match t.get::<Option<String>>(k.as_str())? {
                    Some(prev) if k == "set-cookie" => format!("{prev}\n{v}"),
                    Some(prev) => format!("{prev}, {v}"),
                    None => v.clone(),
                };
                t.set(k.as_str(), joined)?;
            }
            Ok(t)
        };
        let r = match result {
            Ok((status, body, h)) => headers_table(&h).and_then(|ht| {
                // 304 Not Modified is a success for conditional requests.
                if (200..300).contains(&status) || status == 304 {
                    f.call::<()>((body, LuaValue::Nil, status, ht))
                } else {
                    f.call::<()>((body, format!("HTTP {status}"), status, ht))
                }
            }),
            Err(e) => f.call::<()>((LuaValue::Nil, e, LuaValue::Nil, LuaValue::Nil)),
        };
        self.touched();
        if let Err(e) = r {
            self.report(format!("wlens.fetch callback: {e}"));
        }
    }

    pub fn has_hooks(&self, event: &str) -> bool {
        self.shared
            .borrow()
            .hooks
            .get(event)
            .is_some_and(|h| !h.is_empty())
    }

    /// Call every handler for `event`, passing the current image info.
    pub fn emit(&self, event: &str) {
        let (hooks, info) = {
            let sh = self.shared.borrow();
            (
                sh.hooks.get(event).cloned().unwrap_or_default(),
                sh.info.clone(),
            )
        };
        if hooks.is_empty() {
            return;
        }
        let arg = match &info {
            Some(i) => match info_table(&self.lua, i) {
                Ok(t) => LuaValue::Table(t),
                Err(e) => return log::error!("{e}"),
            },
            None => LuaValue::Nil,
        };
        for f in hooks {
            self.call_with(&f, arg.clone(), &format!("'{event}' hook"));
        }
    }

    /// The current image info as a Lua table (or nil).
    pub fn info_value(&self) -> LuaValue {
        let info = self.shared.borrow().info.clone();
        match info.map(|i| info_table(&self.lua, &i)) {
            Some(Ok(t)) => LuaValue::Table(t),
            _ => LuaValue::Nil,
        }
    }

    /// Take timers that are due, in deadline order.
    pub fn due_timers(&self, now: std::time::Instant) -> Vec<Function> {
        let mut sh = self.shared.borrow_mut();
        let mut due: Vec<Timer> = Vec::new();
        let mut i = 0;
        while i < sh.timers.len() {
            if sh.timers[i].at <= now {
                due.push(sh.timers.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|t| t.at);
        due.into_iter().map(|t| t.f).collect()
    }

    pub fn next_timer(&self) -> Option<std::time::Instant> {
        self.shared.borrow().timers.iter().map(|t| t.at).min()
    }

    /// Call `wlens.<name>(arg)` if the user/preset defined it (e.g. `toast`).
    pub fn call_api(&self, name: &str, arg: &str) {
        let f = self
            .lua
            .globals()
            .get::<Table>("wlens")
            .and_then(|w| w.get::<Option<Function>>(name));
        if let Ok(Some(f)) = f {
            self.call_with(&f, arg, &format!("wlens.{name}"));
        }
    }

    pub fn lua_string(&self, s: &str) -> mlua::Value {
        self.lua
            .create_string(s)
            .map(mlua::Value::String)
            .unwrap_or(mlua::Value::Nil)
    }

    /// A script error: logged, and shown on screen (a toast) so a broken
    /// script doesn't just silently do nothing.
    pub fn report(&self, msg: String) {
        log::error!("lua: {msg}");
        self.shared.borrow_mut().errors.push(msg);
    }

    pub fn call(&self, f: &Function) {
        self.call_with(f, (), "");
    }

    /// Call a script function; afterwards the UI re-evaluates (whatever
    /// state the function changed shows up without a manual refresh).
    pub fn call_with(&self, f: &Function, arg: impl mlua::IntoLuaMulti, what: &str) {
        let r = f.call::<()>(arg);
        self.touched();
        if let Err(e) = r {
            self.report(if what.is_empty() {
                format!("{e}")
            } else {
                format!("{what}: {e}")
            });
        }
    }

    /// Script code ran: re-evaluate the UI.
    pub fn touched(&self) {
        let mut sh = self.shared.borrow_mut();
        sh.ui_dirty = true;
        sh.generation += 1;
    }
}

fn info_table(lua: &Lua, i: &ImageInfo) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("path", i.path.as_str())?;
    t.set("name", i.name.as_str())?;
    t.set("dir", i.dir.as_str())?;
    t.set("remote", i.remote)?;
    t.set("index", i.index + 1)?;
    t.set("count", i.count)?;
    t.set("width", i.width)?;
    t.set("height", i.height)?;
    t.set("zoom", i.zoom)?;
    t.set("fitted", i.fitted)?;
    t.set("zoomed_in", i.zoomed_in)?;
    t.set("has_prev", i.has_prev)?;
    t.set("has_next", i.has_next)?;
    t.set("mode", i.mode)?;
    t.set("container", i.container)?;
    t.set("count_in", i.item_count.as_deref())?;
    t.set("meta", i.meta.clone())?;
    t.set("loading", i.loading)?;
    t.set("depth", i.depth)?;
    t.set("level", i.level.as_deref())?;
    t.set("frames", i.frames)?;
    t.set("video", i.video)?;
    t.set("position", i.position)?;
    t.set("duration", i.duration)?;
    t.set("paused", i.paused)?;
    t.set("volume", i.volume)?;
    t.set("muted", i.muted)?;
    t.set("frame", i.frame)?;
    t.set("size", i.file_size)?;
    Ok(t)
}

fn parse_items(list: &Table) -> mlua::Result<Vec<Item>> {
    let mut out = Vec::new();
    for v in list.sequence_values::<LuaValue>() {
        out.push(match v? {
            LuaValue::String(s) => Item::new(Source::parse(&s.to_str()?)),
            LuaValue::Table(t) => {
                let src: String = t
                    .get("src")
                    .or_else(|_| t.get("url"))
                    .or_else(|_| t.get("path"))
                    .map_err(|_| lua_err("item tables need a `src` (path or URL)"))?;
                let thumb: Option<String> = t.get("thumb")?;
                let open = match t.get::<LuaValue>("open")? {
                    LuaValue::Nil => None,
                    LuaValue::Function(f) => Some(Action::Lua(f)),
                    LuaValue::String(s) => {
                        Some(Action::Command(s.to_str()?.parse().map_err(lua_err)?))
                    }
                    _ => return Err(lua_err("item `open` must be a function or command string")),
                };
                Item {
                    src: Source::parse(&src),
                    label: t.get("label")?,
                    thumb: thumb.map(|s| Source::parse(&s)),
                    container: t
                        .get::<Option<bool>>("container")?
                        .unwrap_or(open.is_some()),
                    open,
                    count: match t.get::<LuaValue>("count")? {
                        LuaValue::Nil => None,
                        v => Some(to_opt_string(&v)?),
                    },
                    archive_folder: None,
                    icon: t.get("icon")?,
                    meta: t.get("meta")?,
                    video: t.get::<Option<bool>>("video")?.unwrap_or(false),
                }
            }
            other => {
                return Err(lua_err(format!(
                    "items must be strings or tables, got {}",
                    other.type_name()
                )));
            }
        });
    }
    Ok(out)
}

/// `{a = {b = 1}, c = {"x", "y"}}` -> `[("a.b", "1"), ("c.1", "x"), ("c.2", "y")]`
fn flatten(prefix: &str, v: LuaValue, out: &mut Vec<(String, String)>) -> mlua::Result<()> {
    match v {
        LuaValue::Table(t) => {
            for pair in t.pairs::<LuaValue, LuaValue>() {
                let (k, v) = pair?;
                let k = match k {
                    LuaValue::String(s) => s.to_str()?.to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    other => {
                        return Err(lua_err(format!(
                            "theme: bad key type {}",
                            other.type_name()
                        )));
                    }
                };
                let path = if prefix.is_empty() {
                    k
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&path, v, out)?;
            }
        }
        other => out.push((prefix.to_string(), to_opt_string(&other)?)),
    }
    Ok(())
}
