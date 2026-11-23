mod app;
mod files;
mod geom;
mod io;
mod lua;
mod model;
mod render;
mod ui;

use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;

/// A lightweight, scriptable image viewer for Wayland.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Images, videos, archives, directories or http(s) URLs. Use `-` to read them from stdin.
    files: Vec<PathBuf>,
    /// Recurse into directories.
    #[arg(short, long)]
    recursive: bool,
    /// When given a single file, also browse the other images in its directory.
    #[arg(short = 'S', long)]
    siblings: bool,
    /// Start at this (1-based) index.
    #[arg(short = 'n', long, default_value_t = 1)]
    start: usize,
    /// Start fullscreen.
    #[arg(short, long)]
    fullscreen: bool,
    /// Config file (default: $XDG_CONFIG_HOME/wlens/init.lua).
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Browse folders with openable subfolder and archive tiles in the gallery.
    #[arg(short, long)]
    browse: bool,
    /// Open media from all subfolders as one list. Combine with --browse for a thumbnail grid.
    #[arg(long)]
    flat: bool,
    /// Theme to use: minimal, simple, lightbox, or one of yours in
    /// ~/.config/wlens/themes/. Overrides any wlens.use() in configs.
    #[arg(short, long, env = "WLENS_THEME")]
    theme: Option<String>,
    /// Colour palette for the theme: a palette theme in ~/.config/wlens/themes
    /// (e.g. gruvbox, gruvbox:light) or a path to a palette .lua/.json file.
    #[arg(short, long, env = "WLENS_PALETTE")]
    palette: Option<String>,
    /// List the palettes (built-in and in ~/.config/wlens/themes) and exit.
    #[arg(long)]
    list_palettes: bool,
    /// Ignore the user config; only built-in defaults.
    #[arg(long)]
    no_config: bool,
    /// Send a command (e.g. `next`, `zoom in`, `get current`) to a running
    /// wlens and exit. Goes to $WLENS_SOCKET, else the newest viewer.
    #[arg(long, value_name = "COMMAND", num_args = 1.., conflicts_with = "files")]
    msg: Option<Vec<String>>,
    /// The socket for --msg (default: $WLENS_SOCKET, else the newest viewer).
    #[arg(long, requires = "msg")]
    socket: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    // GStreamer's GL plugins probe EGL when they load; Mesa's warnings about
    // it are noise here (we never use GStreamer GL).
    if std::env::var_os("EGL_LOG_LEVEL").is_none() {
        // SAFETY: single-threaded — nothing else has started yet.
        unsafe { std::env::set_var("EGL_LOG_LEVEL", "fatal") };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("wlens=info,warn"))
        .init();
    let cli = Cli::parse();

    if let Some(words) = &cli.msg {
        let socket = io::ipc::find_socket(cli.socket.as_deref())?;
        if let Some(data) = io::ipc::send(&socket, &words.join(" "))? {
            println!("{data}");
        }
        return Ok(());
    }

    // Bind the control socket early, so everything we start (including
    // `wlens.spawn` from the config) sees $WLENS_SOCKET.
    let ipc = io::ipc::Server::bind();
    if let Some(s) = &ipc {
        // SAFETY: single-threaded — nothing else has started yet.
        unsafe { std::env::set_var("WLENS_SOCKET", s.path()) };
    }

    if cli.list_palettes {
        let script = lua::Script::new().map_err(|e| anyhow::anyhow!("init lua: {e}"))?;
        let names: Vec<String> = script
            .lua
            .load("return wlens.widgets.palette_names()")
            .eval()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("{}", names.join("\n"));
        return Ok(());
    }

    let script = lua::Script::new().map_err(|e| anyhow::anyhow!("init lua: {e}"))?;
    script
        .set_args(
            cli.theme.as_deref().filter(|t| !t.is_empty()),
            cli.palette.as_deref().filter(|p| !p.is_empty()),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // $WLENS_PROXY: send everything through a proxy (configs can set wlens.opt.proxy too).
    if let Ok(p) = std::env::var("WLENS_PROXY")
        && let Err(e) = script.shared.borrow_mut().settings.set("proxy", &p)
    {
        log::warn!("WLENS_PROXY: {e}");
    }
    // Remembered state is kept per script: the config file (or folder) in use.
    {
        let config = if cli.no_config {
            None
        } else {
            cli.config
                .clone()
                .or_else(|| dirs::config_dir().map(|d| d.join("wlens/init.lua")))
        };
        script.shared.borrow_mut().script_id = config
            .filter(|p| p.exists())
            .map(|p| {
                std::fs::canonicalize(&p)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "default".into());
    }
    script
        .run_str("=default.lua", lua::DEFAULT_CONFIG)
        .map_err(|e| anyhow::anyhow!("built-in default config: {e}"))?;
    if !cli.no_config {
        let path = cli
            .config
            .clone()
            .or_else(|| dirs::config_dir().map(|d| d.join("wlens/init.lua")));
        if let Some(path) = path.filter(|p| p.exists()) {
            log::info!("config: {}", path.display());
            // A broken config shouldn't stop you looking at pictures.
            if let Err(e) = script.run_file(&path) {
                log::error!("{e}");
            }
        } else if cli.config.is_some() {
            bail!("config file not found");
        }
    }

    // Flat view determines which items to gather. Apply the other remembered
    // options after the browser installs its theme defaults.
    let mut options = io::state::section(&script.shared.borrow().script_id, "options");
    if let Some(serde_json::Value::String(value)) = options.remove("gallery.flat")
        && let Err(err) = script
            .shared
            .borrow_mut()
            .settings
            .set("gallery.flat", &value)
    {
        log::debug!("remembered option gallery.flat: {err}");
    }

    let single = match cli.files.as_slice() {
        [p] => Some(p),
        _ => None,
    };
    let archive = single.filter(|p| io::archive::is_archive(p) && p.is_file());
    let directory = single
        .filter(|p| p.is_dir())
        .map(PathBuf::as_path)
        .or_else(|| cli.files.is_empty().then_some(std::path::Path::new(".")));
    let browse_dir = directory.is_some()
        && !cli.flat
        && (cli.browse || (single.is_some() && !cli.recursive && cli.start == 1));
    let flat = {
        let mut sh = script.shared.borrow_mut();
        sh.settings.flat |= cli.flat
            || (directory.is_some() && cli.recursive && !cli.browse)
            || (archive.is_some() && cli.start > 1 && !cli.browse);
        sh.settings.flat
    };
    let mut browser = directory
        .map(|dir| app::Browser::new(app::BrowserLocation::Directory(dir.to_owned()), flat));
    let mut start_in_grid = cli.browse || browse_dir;
    let (mut items, mut start) = if let Some(path) = archive {
        let root = model::archive_folder::ArchiveFolder::opened(
            path,
            io::archive::list_entries(path, true)?,
        );
        let folders = root.items();
        start_in_grid |= !cli.flat && cli.start == 1 && folders.iter().any(|item| item.container);
        let items = if flat { root.flat_items() } else { folders };
        let start = cli
            .start
            .saturating_sub(1)
            .min(items.len().saturating_sub(1));
        browser = Some(app::Browser::new(app::BrowserLocation::Archive(root), flat));
        (items, start)
    } else if browse_dir && !flat {
        (Vec::new(), 0)
    } else {
        files::gather(&files::Request {
            inputs: &cli.files,
            recursive: cli.recursive || flat,
            siblings: cli.siblings,
            start: cli.start,
        })?
    };
    if browse_dir && !flat {
        let dir = cli
            .files
            .first()
            .filter(|p| p.is_dir())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let browse: mlua::Function = script
            .lua
            .load("return function(p) wlens.widgets.browse(p) end")
            .eval()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        browse
            .call::<()>(dir.to_string_lossy().as_ref())
            .map_err(|e| anyhow::anyhow!("browse: {e}"))?;
    } else if start_in_grid || (browser.is_some() && items.is_empty()) {
        script
            .run_str(
                "=browse",
                "wlens.theme { gallery = { labels = true } }; wlens.gallery('open')",
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    // The config may supply items itself (wlens.set_items), or fetch them
    // once running (a "start" hook). Only give up if neither can happen.
    {
        let mut sh = script.shared.borrow_mut();
        // Pushes before startup just become the starting list.
        for op in std::mem::take(&mut sh.pending_items) {
            match op {
                lua::ItemsOp::Add(list) => items.extend(list),
                lua::ItemsOp::Set(list, idx) | lua::ItemsOp::Push(list, idx, _) => {
                    browser = None;
                    items = list;
                    start = idx.unwrap_or(0);
                }
                lua::ItemsOp::Directory(dir) => {
                    browser = Some(app::Browser::new(
                        app::BrowserLocation::Directory(dir),
                        false,
                    ))
                }
                lua::ItemsOp::Pop => {}
                lua::ItemsOp::More(m) => sh.more = m,
            }
        }
        if items.is_empty()
            && browser.is_none()
            && !browse_dir
            && sh.hooks.get("start").is_none_or(|h| h.is_empty())
        {
            bail!("no images to show");
        }
    }

    // Options you changed last time (menus, keys) win over the config's.
    {
        let mut sh = script.shared.borrow_mut();
        for (k, v) in options {
            if let Some(v) = v.as_str()
                && let Err(e) = sh.settings.set(&k, v)
            {
                log::debug!("remembered option {k}: {e}");
            }
        }
        sh.started = true;
    }

    // Trim the download cache in the background (configs may have changed the limit).
    let limit = script.shared.borrow().settings.remote_cache_mb as u64;
    let _ = std::thread::Builder::new()
        .name("wlens-cache-prune".into())
        .spawn(move || io::remote::prune_cache(limit << 20));

    let event_loop = winit::event_loop::EventLoop::<app::UserEvent>::with_user_event().build()?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(event_loop.owned_display_handle()),
    ));
    if let Some(browser) = &browser {
        let path = match &browser.location {
            app::BrowserLocation::Directory(dir) => Some(dir),
            app::BrowserLocation::Archive(_) => archive,
        };
        script.shared.borrow_mut().level = path.map(|p| files::location_name(p));
    }
    let mut app = app::App::new(
        items,
        start,
        cli.fullscreen,
        script,
        event_loop.create_proxy(),
        instance,
    );
    app.browser = browser;
    let socket = ipc.map(|s| s.serve(event_loop.create_proxy()));
    event_loop.run_app(&mut app)?;
    drop(socket); // remove the socket file before exiting
    if app.startup_failed {
        // Already logged (and notified); just fail.
        std::process::exit(1);
    }
    Ok(())
}
