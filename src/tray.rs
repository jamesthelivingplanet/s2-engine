//! The system tray icon, and the systemd user service that shows it at login.

use crate::config::Config;
use crate::{Result, paths, server};
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::Duration;

#[derive(clap::Subcommand)]
pub enum Action {
    /// Install and start a systemd user service that shows the icon at login
    Setup,
    /// Undo `setup`
    Remove,
}

const UNIT: &str = "s2-tray.service";

pub fn run(action: Option<Action>) -> Result<()> {
    match action {
        None => show(),
        Some(Action::Setup) => setup(),
        Some(Action::Remove) => remove(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Stopped,
    Starting(String),
    Running { label: String, port: u16 },
}

/// Everything the icon and menu are drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub state: State,
    /// Profile name and whether its model file is present.
    pub profiles: Vec<(String, bool)>,
    pub error: Option<String>,
}

pub fn snapshot() -> Snapshot {
    let (config, error) = match Config::load() {
        Ok(config) => (Some(config), None),
        Err(e) => (None, Some(e)),
    };
    let state = match server::running(config.as_ref()) {
        None => State::Stopped,
        Some(run) => {
            let label = run.profile.unwrap_or(run.alias);
            match run.port {
                Some(port) if !server::healthy(port) => State::Starting(label),
                port => State::Running {
                    label,
                    port: port.unwrap_or(8080),
                },
            }
        }
    };
    let profiles = config
        .iter()
        .flat_map(|c| &c.profiles)
        .map(|p| (p.name.clone(), p.model_path().is_file()))
        .collect();
    Snapshot {
        state,
        profiles,
        error,
    }
}

fn decode(png_bytes: &[u8]) -> ksni::Icon {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("embedded icon is a PNG");
    let mut data = vec![0; reader.output_buffer_size().expect("icon fits in memory")];
    let info = reader.next_frame(&mut data).expect("embedded icon decodes");
    assert_eq!(info.color_type, png::ColorType::Rgba, "icons are RGBA");
    data.truncate(info.buffer_size());
    // The protocol wants ARGB.
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    ksni::Icon {
        width: info.width as i32,
        height: info.height as i32,
        data,
    }
}

macro_rules! icon_set {
    ($state:literal) => {
        LazyLock::new(|| {
            vec![
                decode(include_bytes!(concat!(
                    "../assets/icons/",
                    $state,
                    "-22.png"
                ))),
                decode(include_bytes!(concat!(
                    "../assets/icons/",
                    $state,
                    "-32.png"
                ))),
                decode(include_bytes!(concat!(
                    "../assets/icons/",
                    $state,
                    "-48.png"
                ))),
            ]
        })
    };
}

static STOPPED: LazyLock<Vec<ksni::Icon>> = icon_set!("stopped");
static STARTING: LazyLock<Vec<ksni::Icon>> = icon_set!("starting");
static RUNNING: LazyLock<Vec<ksni::Icon>> = icon_set!("running");

struct Tray {
    seen: Snapshot,
    quit: Arc<AtomicBool>,
}

fn notify(message: &str) {
    eprintln!("{message}");
    let _ = Command::new("notify-send")
        .args(["S2 Engine", message])
        .status();
}

/// Menu actions must return at once, so the work happens on its own thread.
fn start_profile(name: String) {
    thread::spawn(move || {
        let result = Config::load().and_then(|config| {
            let profile = config.find(&name)?;
            server::start(&config, profile, &mut |line| println!("{line}"))
        });
        if let Err(e) = result {
            notify(&e);
        }
    });
}

fn stop_model() {
    thread::spawn(|| match server::stop(Config::load().ok().as_ref()) {
        Ok(message) => println!("{message}"),
        Err(e) => notify(&e),
    });
}

fn open_chat(port: u16) {
    thread::spawn(move || {
        let url = format!("http://127.0.0.1:{port}");
        if !Command::new("xdg-open")
            .arg(&url)
            .status()
            .is_ok_and(|s| s.success())
        {
            notify(&format!("could not open {url}"));
        }
    });
}

/// A single underscore in a menu label marks a shortcut key, so double them.
fn label(text: &str) -> String {
    text.replace('_', "__")
}

fn item(
    text: String,
    enabled: bool,
    action: impl Fn(&mut Tray) + Send + 'static,
) -> MenuItem<Tray> {
    StandardItem {
        label: label(&text),
        enabled,
        activate: Box::new(action),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "s2-engine".into()
    }

    fn title(&self) -> String {
        "S2 Engine".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        match self.seen.state {
            State::Stopped => STOPPED.clone(),
            State::Starting(_) => STARTING.clone(),
            State::Running { .. } => RUNNING.clone(),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "S2 Engine".into(),
            description: headline(&self.seen.state),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = vec![item(headline(&self.seen.state), false, |_| {})];
        if self.seen.error.is_some() {
            menu.push(item("Config problem: run `s2 list`".into(), false, |_| {}));
        }
        menu.push(MenuItem::Separator);
        let profiles = self
            .seen
            .profiles
            .iter()
            .map(|(name, present)| {
                let name = name.clone();
                let suffix = if *present { "" } else { " (missing)" };
                item(format!("{name}{suffix}"), *present, move |_| {
                    start_profile(name.clone())
                })
            })
            .collect();
        menu.push(
            SubMenu {
                label: "Start".into(),
                enabled: !self.seen.profiles.is_empty(),
                submenu: profiles,
                ..Default::default()
            }
            .into(),
        );
        let running = self.seen.state != State::Stopped;
        menu.push(item("Stop".into(), running, |_| stop_model()));
        let port = match self.seen.state {
            State::Running { port, .. } => Some(port),
            _ => None,
        };
        menu.push(item(
            "Open chat in browser".into(),
            port.is_some(),
            move |_| {
                if let Some(port) = port {
                    open_chat(port);
                }
            },
        ));
        menu.push(MenuItem::Separator);
        menu.push(item("Quit".into(), true, |tray| {
            tray.quit.store(true, Ordering::SeqCst)
        }));
        menu
    }

    // Stay alive while the bar restarts.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        true
    }
}

fn headline(state: &State) -> String {
    match state {
        State::Stopped => "No model running".into(),
        State::Starting(name) => format!("Starting: {name}"),
        State::Running { label, port } => format!("Running: {label} (:{port})"),
    }
}

fn show() -> Result<()> {
    let quit = Arc::new(AtomicBool::new(false));
    let tray = Tray {
        seen: snapshot(),
        quit: quit.clone(),
    };
    let handle = tray
        .assume_sni_available(true)
        .spawn()
        .map_err(|e| format!("cannot show the tray icon: {e}"))?;
    let mut ticks = 0;
    while !quit.load(Ordering::SeqCst) && !handle.is_closed() {
        thread::sleep(Duration::from_millis(200));
        ticks += 1;
        if ticks >= 10 {
            ticks = 0;
            let now = snapshot();
            handle.update(|tray| tray.seen = now);
        }
    }
    handle.shutdown().wait();
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let program = std::env::var("S2_SYSTEMCTL").unwrap_or_else(|_| "systemctl".into());
    let status = Command::new(&program)
        .arg("--user")
        .args(args)
        .status()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl --user {} failed", args.join(" ")))
    }
}

pub fn unit_text(program: &str) -> String {
    // KillMode=process: stopping the icon must leave the model server running.
    format!(
        "[Unit]
Description=S2 Engine tray icon
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=\"{program}\" tray
Restart=on-failure
RestartSec=5
KillMode=process
Environment=XDG_RUNTIME_DIR=%t

[Install]
WantedBy=graphical-session.target
"
    )
}

fn setup() -> Result<()> {
    let program = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let unit = paths::systemd_user_dir().join(UNIT);
    paths::write_file(&unit, &unit_text(&program.to_string_lossy()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", UNIT])?;
    // Restart so that running setup again picks up a new version.
    systemctl(&["restart", UNIT])?;
    println!("tray icon set up: {}", unit.display());
    Ok(())
}

fn remove() -> Result<()> {
    let unit = paths::systemd_user_dir().join(UNIT);
    if !unit.exists() {
        println!("tray icon is not set up");
        return Ok(());
    }
    systemctl(&["disable", "--now", UNIT])?;
    std::fs::remove_file(&unit).map_err(|e| format!("cannot delete {}: {e}", unit.display()))?;
    systemctl(&["daemon-reload"])?;
    println!("tray icon removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icons_decode_at_their_sizes() {
        for set in [&STOPPED, &STARTING, &RUNNING] {
            let sizes: Vec<_> = set
                .iter()
                .map(|i| (i.width, i.height, i.data.len()))
                .collect();
            assert_eq!(sizes, [(22, 22, 1936), (32, 32, 4096), (48, 48, 9216)]);
        }
    }
}
