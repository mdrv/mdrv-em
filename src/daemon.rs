//! Daemon state, layer-shell window lifecycle, and the control socket.

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{Receiver, Sender};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::prelude::*;
use gpui::{
    px, size, App, Bounds, Global, SharedString, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};

use crate::conf::Conf;
use crate::state::Recents;
use crate::ui::PickerView;

pub const APP: &str = "mdrv-em";

/// App-wide state shared by the daemon loop and the panel view.
pub struct Daemon {
    pub conf: Conf,
    /// Resolved emoji font family (custom paths are registered up front).
    pub emoji_font: SharedString,
    pub recents: Recents,
    pub open: bool,
    pub handle: Option<WindowHandle<PickerView>>,
    /// Background search worker link (engine lives on the thread).
    pub search_tx: std::sync::mpsc::Sender<crate::search::SearchReq>,
    pub search_rx: std::sync::mpsc::Receiver<crate::search::SearchResp>,
    /// Incremented per query; stale answers are dropped by seq check.
    pub search_seq: u64,
}

impl Global for Daemon {}

/// Create the persistent layer-shell window: hidden, keyboard-less. It is
/// never destroyed — show/hide toggle visibility + keyboard interactivity
/// through `PickerView::apply_visibility` (upperadd's overlay lifecycle).
pub fn create_window(cx: &mut App) {
    if cx.global::<Daemon>().handle.is_some() {
        return;
    }
    let opts = window_options(&cx.global::<Daemon>().conf);
    match cx.open_window(opts, |_, cx| cx.new(PickerView::new)) {
        Ok(h) => cx.global_mut::<Daemon>().handle = Some(h),
        Err(e) => eprintln!("{APP}: open window: {e}"),
    }
}

pub fn show(cx: &mut App) {
    let d = cx.global_mut::<Daemon>();
    d.open = true;
    if let Some(h) = d.handle.clone() {
        let _ = h.update(cx, |v, window, cx| v.apply_visibility(true, window, cx));
    }
}

pub fn hide(cx: &mut App) {
    let d = cx.global_mut::<Daemon>();
    d.open = false;
    if let Some(h) = d.handle.clone() {
        let _ = h.update(cx, |v, window, cx| v.apply_visibility(false, window, cx));
    }
}

pub fn toggle(cx: &mut App) {
    if cx.global::<Daemon>().open {
        hide(cx);
    } else {
        show(cx);
    }
}

fn window_options(_conf: &Conf) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(0.), px(0.)),
        })),
        window_background: WindowBackgroundAppearance::Transparent,
        focus: false,
        show: false,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: APP.into(),
            layer: Layer::Overlay,
            // §16.3: all four anchors fill the output — 0×0 bounds so the
            // compositor's anchors win; the panel is positioned inside.
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            exclusive_zone: Some(px(-1.)),
            exclusive_edge: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        ..Default::default()
    }
}

/// Commands from the control socket to the app's async poll loop.
pub enum Cmd {
    Show,
    Hide,
    Toggle,
    Stop,
}

/// Control socket: `mdrv-em <verb>` connects and sends one line.
/// Commands are forwarded over an mpsc channel (gpui app state is not
/// `Send`, so the thread must not touch it directly).
pub fn spawn_ctrl_socket(cmd_tx: Sender<Cmd>) {
    let path = sock_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{APP}: bind {}: {e}", path.display());
            return;
        }
    };
    let _ = std::thread::Builder::new()
        .name(format!("{APP}-ctrl"))
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let cmd = match line.trim() {
                    "show" => Some(Cmd::Show),
                    "hide" => Some(Cmd::Hide),
                    "toggle" => Some(Cmd::Toggle),
                    "stop" | "quit" => Some(Cmd::Stop),
                    _ => None, // liveness ping
                };
                if let Some(cmd) = cmd {
                    if cmd_tx.send(cmd).is_err() {
                        break;
                    }
                }
            }
        });
}

/// Drain pending socket commands; called from the app's async poll loop.
/// Returns false once the channel is dead (app shutting down).
pub fn poll_commands(rx: &Receiver<Cmd>, app: &mut gpui::AsyncApp) -> bool {
    loop {
        match rx.try_recv() {
            Ok(Cmd::Show) => app.update(show),
            Ok(Cmd::Hide) => app.update(hide),
            Ok(Cmd::Toggle) => app.update(toggle),
            Ok(Cmd::Stop) => {
                app.update(|cx: &mut App| cx.quit());
                let _ = std::fs::remove_file(sock_path());
                process::exit(0);
            }
            Err(_) => return true,
        }
    }
}

pub fn sock_path() -> PathBuf {
    runtime_dir().join(format!("{APP}.sock"))
}

/// XDG_RUNTIME_DIR, falling back to /run/user/<uid>.
pub fn runtime_dir() -> PathBuf {
    if let Ok(d) = std::env::var("XDG_RUNTIME_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Some(uid) = uid() {
        let p = PathBuf::from(format!("/run/user/{uid}"));
        if p.is_dir() {
            return p;
        }
    }
    std::env::temp_dir()
}

pub fn uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|l| l.starts_with("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Used by client verbs (`toggle`/`show`/`hide`/`stop`).
pub fn send_line(cmd: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut stream = UnixStream::connect(sock_path())?;
    stream.set_write_timeout(Some(std::time::Duration::from_millis(300)))?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")
}
