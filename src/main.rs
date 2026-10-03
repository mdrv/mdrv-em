//! mdrv-em — semantic emoji picker (Hyprland layer-shell panel).
//!
//! Daemon verbs: `run [--hidden]` (systemd unit uses --hidden).
//! Client verbs: `toggle` (default), `show`, `hide`, `stop`.
//! With no daemon up, `toggle`/`show` spawn a fresh one that shows
//! immediately (clock-style); `hide` is a no-op.

mod conf;
mod daemon;
mod emoji_data;
mod search;
mod state;
mod ui;
mod vectors_data;

use std::borrow::Cow;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use std::{env, process};

use gpui::{App, SharedString};
use gpui_platform::application;

use daemon::Daemon;

fn main() {
    let verb = env::args().nth(1).unwrap_or_else(|| "toggle".into());
    match verb.as_str() {
        "run" => app_main(env::args().any(|a| a == "--hidden")),
        "toggle" | "show" => {
            let cmd = if verb == "show" { "show" } else { "toggle" };
            if daemon::send_line(cmd).is_err() {
                spawn_detached("run"); // no daemon: summon one, panel shows
            }
        }
        "hide" => {
            let _ = daemon::send_line("hide");
        }
        "stop" => {
            let _ = daemon::send_line("stop");
        }
        "help" | "--help" | "-h" => print_help(),
        other => {
            eprintln!("mdrv-em: unknown verb: {other}");
            print_help();
            process::exit(2);
        }
    }
}

fn app_main(hidden: bool) {
    ensure_session_env();
    let conf = conf::Conf::load();
    // Search worker thread: owns the ONNX session so embedding never
    // blocks the UI thread, and loads in parallel with gpui startup.
    let (search_tx, search_rx) = search::spawn_worker(
        conf.search.fallback_locales.clone(),
        conf.search.max_results,
    );
    application().run(move |cx: &mut App| {
        let emoji_font = resolve_font(cx, &conf.font.emoji, "Twemoji");
        cx.set_global(Daemon {
            recents: state::Recents::load(),
            emoji_font,
            conf,
            open: false,
            handle: None,
            search_tx,
            search_rx,
            search_seq: 0,
        });
        let (tx, rx) = std::sync::mpsc::channel();
        daemon::spawn_ctrl_socket(tx);
        cx.spawn(async move |app: &mut gpui::AsyncApp| {
            let bg = app.update(|cx| cx.background_executor().clone());
            loop {
                if !daemon::poll_commands(&rx, app) {
                    return;
                }
                // Relay the newest answer for the current query seq.
                let fresh = app.update(|cx| {
                    let d = cx.global_mut::<Daemon>();
                    let mut fresh = None;
                    while let Ok((seq, results)) = d.search_rx.try_recv() {
                        if seq == d.search_seq {
                            fresh = Some(results);
                        }
                    }
                    fresh
                });
                if let Some(results) = fresh {
                    app.update(|cx| {
                        if let Some(h) = cx.global::<Daemon>().handle.clone() {
                            let _ = h.update(cx, |v, _w, cx| v.apply_results(results, cx));
                        }
                    });
                }
                bg.timer(Duration::from_millis(30)).await;
            }
        })
        .detach();
        daemon::create_window(cx);
        if !hidden {
            daemon::show(cx);
        }
    });
}

/// Family name, or an absolute font path registered with the text
/// system (family detected by diffing `all_font_names`).
fn resolve_font(cx: &App, spec: &str, fallback: &str) -> SharedString {
    let spec = if spec.is_empty() { fallback } else { spec };
    let p = Path::new(spec);
    if p.is_absolute() && p.exists() {
        match std::fs::read(p) {
            Ok(bytes) => {
                let ts = cx.text_system();
                let before = ts.all_font_names();
                if ts
                    .add_fonts(vec![Cow::Owned(bytes)])
                    .is_ok()
                {
                    if let Some(name) = ts
                        .all_font_names()
                        .into_iter()
                        .find(|f| !before.contains(f))
                    {
                        return name.into();
                    }
                }
            }
            Err(e) => eprintln!("mdrv-em: font {}: {e}", p.display()),
        }
    }
    SharedString::from(spec.to_string())
}

/// User-session fallbacks so the daemon also works when spawned straight
/// from a keybind without a login-shell environment.
fn ensure_session_env() {
    if env::var("WAYLAND_DISPLAY").map_or(true, |v| v.is_empty()) {
        env::set_var("WAYLAND_DISPLAY", "wayland-1");
    }
    if env::var("XDG_RUNTIME_DIR").map_or(true, |v| v.is_empty()) {
        if let Some(uid) = daemon::uid() {
            env::set_var("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
        }
    }
}

/// Spawn `mdrv-em <verb>` detached from the current process (keybind /
/// client verb path): new process group, stdio to the void.
fn spawn_detached(verb: &str) {
    use std::os::unix::process::CommandExt;
    let exe = match env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("mdrv-em: current_exe: {e}");
            return;
        }
    };
    let mut cmd = Command::new(exe);
    cmd.arg(verb).process_group(0);
    if env::var("WAYLAND_DISPLAY").map_or(true, |v| v.is_empty()) {
        cmd.env("WAYLAND_DISPLAY", "wayland-1");
    }
    if env::var("XDG_RUNTIME_DIR").map_or(true, |v| v.is_empty()) {
        if let Some(uid) = daemon::uid() {
            cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
        }
    }
    if let Err(e) = cmd
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .spawn()
    {
        eprintln!("mdrv-em: spawn: {e}");
    }
}

fn print_help() {
    println!(
        "mdrv-em — semantic emoji picker

verbs:
  run [--hidden]  start the daemon (hidden = no panel at startup)
  toggle          summon/dismiss the panel (default; spawns a daemon if none)
  show            summon only
  hide            dismiss only
  stop            stop the daemon

config:  ~/.config/mdrv-em/config.toml
state:   ~/.local/state/mdrv-em/recents.json"
    );
}
