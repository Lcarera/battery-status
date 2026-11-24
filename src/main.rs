#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod app;
mod config;
mod device;
mod icon;
mod notify;
mod system;
mod tray;
mod watcher;

use ::notify::{RecursiveMode, Watcher};
use std::cell::OnceCell;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use tray::TrayMsg;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("battery-status")
}

fn init_logging() {
    let dir = data_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(dir.join("battery-status.log")) {
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, simplelog::Config::default(), file);
    }
}

/// Sends a message to the tray thread and wakes it.
#[derive(Clone)]
struct Sink {
    tx: Sender<TrayMsg>,
    hwnd: Arc<AtomicIsize>,
}

impl Sink {
    fn send(&self, msg: TrayMsg) {
        if self.tx.send(msg).is_ok() {
            tray::post_wakeup(self.hwnd.load(Ordering::Acquire));
        }
    }
}

fn load_config(path: &Path) -> config::Config {
    match config::load_or_create(path) {
        Ok((cfg, _created)) => {
            // Point the Run key at this exe on each start. The exe can move,
            // and an uninstall removes the key but keeps config.toml.
            if cfg.start_with_windows {
                if let Err(e) = system::set_autostart(true) {
                    log::error!("autostart not set: {e}");
                }
            }
            cfg
        }
        Err(e) => {
            log::error!("settings file not valid, using defaults: {e}");
            config::Config::default()
        }
    }
}

fn save_autostart_flag(path: &Path, on: bool) {
    let result = fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| config::set_start_flag(&t, on))
        .and_then(|t| fs::write(path, t).map_err(|e| e.to_string()));
    if let Err(e) = result {
        log::error!("settings file not changed (start_with_windows = {on}): {e}");
    }
}

fn open_settings(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    unsafe {
        // Values of 32 or less mean failure, for example no app is set for .toml files.
        if ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL).0 as isize <= 32 {
            log::warn!("no app opens .toml files; using Notepad");
            if ShellExecuteW(None, w!("open"), w!("notepad.exe"), &file, None, SW_SHOWNORMAL).0 as isize <= 32 {
                log::error!("settings file could not be opened: {}", path.display());
            }
        }
    }
}

fn watch_config(path: PathBuf, sink: Sink) -> ::notify::Result<impl Watcher> {
    let target = path.clone();
    let mut w = ::notify::recommended_watcher(move |res: ::notify::Result<::notify::Event>| {
        let Ok(ev) = res else { return };
        if !ev.paths.iter().any(|p| p.file_name() == target.file_name()) {
            return;
        }
        match fs::read_to_string(&target) {
            Ok(text) => sink.send(TrayMsg::ReloadConfig(text)),
            Err(e) => log::warn!("settings file not read: {e}"),
        }
    })?;
    w.watch(path.parent().unwrap_or(Path::new(".")), RecursiveMode::NonRecursive)?;
    Ok(w)
}

fn main() {
    init_logging();
    let Some(_instance) = system::single_instance() else {
        log::info!("another copy runs; exit");
        return;
    };
    log::info!("battery-status started");

    let cfg_path = config::default_path();
    let cfg = load_config(&cfg_path);
    if let Err(e) = notify::register_aumid() {
        log::error!("toast registration failed: {e}");
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let sink = Sink { tx, hwnd: Arc::new(AtomicIsize::new(0)) };
    let watcher_cell: Rc<OnceCell<watcher::WatcherHandle>> = Rc::new(OnceCell::new());

    let toast_sink = sink.clone();
    let resume_cell = watcher_cell.clone();
    let path_for_toggle = cfg_path.clone();
    let path_for_open = cfg_path.clone();
    let hooks = tray::Hooks {
        show_toast: Box::new(move |id, name, percent, level| {
            let s = toast_sink.clone();
            if let Err(e) = notify::show(name, percent, level, move || s.send(TrayMsg::Acknowledge(id))) {
                log::error!("toast failed: {e}");
            }
        }),
        on_resume: Box::new(move || {
            log::info!("resume: reading devices again");
            if let Some(w) = resume_cell.get() {
                w.resync();
            }
        }),
        autostart_enabled: Box::new(system::autostart_enabled),
        set_autostart: Box::new(move |on| {
            if let Err(e) = system::set_autostart(on) {
                log::error!("autostart change failed: {e}");
            }
            save_autostart_flag(&path_for_toggle, on);
        }),
        open_settings: Box::new(move || open_settings(&path_for_open)),
    };

    let tray = match tray::Tray::new(app::App::new(cfg), rx, hooks) {
        Ok(t) => t,
        Err(e) => {
            log::error!("tray window failed: {e}");
            return;
        }
    };
    sink.hwnd.store(tray.hwnd(), Ordering::Release);

    let device_sink = sink.clone();
    match watcher::start(move |ev| device_sink.send(TrayMsg::Device(ev))) {
        Ok(w) => {
            let _ = watcher_cell.set(w);
        }
        Err(e) => log::error!("device watcher failed: {e}"),
    }
    let _config_watcher = watch_config(cfg_path, sink.clone()).map_err(|e| log::error!("settings watch failed: {e}"));

    let code = tray.run();
    log::info!("battery-status exit");
    std::process::exit(code);
}
