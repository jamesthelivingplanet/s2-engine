//! The one running model server: start it, find it, stop it.

use crate::config::{Config, Profile};
use crate::{Result, hermes, paths};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Running {
    pub pid: i32,
    /// The server's `--alias`.
    pub alias: String,
    /// Name of the profile, when it can be told apart.
    pub profile: Option<String>,
    pub port: Option<u16>,
}

#[derive(Serialize, Deserialize)]
struct State {
    pid: i32,
    profile: String,
}

fn state_file() -> std::path::PathBuf {
    paths::state_dir().join("running.json")
}

fn seconds(name: &str, default: u64) -> Duration {
    let value = std::env::var(name).ok().and_then(|v| v.parse().ok());
    Duration::from_secs(value.unwrap_or(default))
}

/// The process arguments, or None once the process is gone (zombies have none).
fn cmdline(pid: i32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.split(|b| *b == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect(),
    )
}

fn read_pid() -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(paths::pidfile())
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (pid > 1).then_some(pid)
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    // The last occurrence wins, as it does for the server.
    let at = args.iter().rposition(|a| a == flag)?;
    args.get(at + 1).map(String::as_str)
}

fn serves(args: &[String], profile: &Profile) -> bool {
    value_after(args, "-m") == Some(profile.model_path().to_string_lossy().as_ref())
        && value_after(args, "-c") == Some(profile.context.to_string().as_str())
}

pub fn running(config: Option<&Config>) -> Option<Running> {
    let pid = read_pid()?;
    let args = cmdline(pid)?;
    let recorded = std::fs::read_to_string(state_file())
        .ok()
        .and_then(|text| serde_json::from_str::<State>(&text).ok())
        .filter(|state| state.pid == pid)
        .map(|state| state.profile);
    // A server started some other way is recognised by its model and context.
    let profile = recorded.or_else(|| {
        let matches: Vec<_> = config?
            .profiles
            .iter()
            .filter(|p| serves(&args, p))
            .collect();
        match matches.as_slice() {
            [only] => Some(only.name.clone()),
            _ => None,
        }
    });
    Some(Running {
        pid,
        alias: value_after(&args, "--alias")
            .unwrap_or_default()
            .to_string(),
        profile,
        port: value_after(&args, "--port").and_then(|p| p.parse().ok()),
    })
}

/// Is the running server exactly this profile, with this model and context?
pub fn running_is(config: &Config, profile: &Profile) -> bool {
    running(Some(config)).is_some_and(|run| {
        run.profile
            .as_deref()
            .is_none_or(|name| name == profile.name)
            && cmdline(run.pid).is_some_and(|args| serves(&args, profile))
    })
}

/// Does the running server have this file on its command line?
pub fn uses(path: &std::path::Path) -> bool {
    let wanted = path.to_string_lossy();
    read_pid()
        .and_then(cmdline)
        .is_some_and(|args| args.iter().any(|a| *a == wanted))
}

pub fn healthy(port: u16) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1)))
        .proxy(None)
        .build()
        .into();
    agent
        .get(format!("http://127.0.0.1:{port}/health"))
        .call()
        .is_ok()
}

fn port_in_use(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok()
}

/// The full command for a profile: program first.
pub fn command_line(config: &Config, profile: &Profile) -> Result<Vec<String>> {
    let mut line = vec![
        config.runtime_of(profile)?,
        "-m".into(),
        profile.model_path().to_string_lossy().into_owned(),
        "-c".into(),
        profile.context.to_string(),
    ];
    line.extend(profile.args.iter().map(|a| paths::expand(a)));
    line.extend([
        "--alias".into(),
        profile.alias.clone(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        profile.port.to_string(),
    ]);
    Ok(line)
}

/// Held while the pid file changes hands, so two starts cannot interleave.
fn lock() -> Result<File> {
    let dir = paths::state_dir();
    let fail = |e: std::io::Error| format!("cannot lock {}: {e}", dir.display());
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let file = File::create(dir.join("lock")).map_err(fail)?;
    file.lock().map_err(fail)?;
    Ok(file)
}

fn alive(pid: i32) -> bool {
    cmdline(pid).is_some()
}

fn stop_locked(config: Option<&Config>) -> Result<String> {
    let message = match running(config) {
        None => "no model running".to_string(),
        Some(run) => {
            // SAFETY: kill takes no pointers; the pid was just read and is above 1.
            unsafe { libc::kill(run.pid, libc::SIGTERM) };
            let deadline = Instant::now() + seconds("S2_STOP_TIMEOUT", 30);
            while alive(run.pid) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(100));
            }
            if alive(run.pid) {
                return Err(format!("pid {0} did not stop; try: kill -9 {0}", run.pid));
            }
            let alias = if run.alias.is_empty() {
                "model"
            } else {
                &run.alias
            };
            format!("{alias} stopped (pid {})", run.pid)
        }
    };
    let _ = std::fs::remove_file(paths::pidfile());
    let _ = std::fs::remove_file(state_file());
    Ok(message)
}

pub fn stop(config: Option<&Config>) -> Result<String> {
    let _guard = lock()?;
    stop_locked(config)
}

fn log_tail() -> String {
    let bytes = std::fs::read(paths::log()).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<_> = text.lines().collect();
    lines[lines.len().saturating_sub(5)..].join("\n")
}

fn spawn(config: &Config, profile: &Profile) -> Result<Child> {
    let line = command_line(config, profile)?;
    let log = paths::log();
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", log.display());
    let out = File::create(&log).map_err(fail)?;
    let err = out.try_clone().map_err(fail)?;
    // Its own process group, so closing the terminal or Ctrl-C leaves it running.
    Command::new(&line[0])
        .args(&line[1..])
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", line[0]))
}

/// Replace whatever is running with `profile` and wait until it answers.
/// Callers decide whether replacing is allowed.
pub fn start(config: &Config, profile: &Profile, say: &mut dyn FnMut(&str)) -> Result<()> {
    let model = profile.model_path();
    if !model.is_file() {
        return Err(format!("model file missing: {}", model.display()));
    }
    let guard = lock()?;
    if running(Some(config)).is_some() {
        say(&stop_locked(Some(config))?);
    }
    if port_in_use(profile.port) {
        return Err(format!(
            "port {} is already in use by a process s2 did not start",
            profile.port
        ));
    }
    let mut child = spawn(config, profile)?;
    let pid = child.id() as i32;
    let state = State {
        pid,
        profile: profile.name.clone(),
    };
    let recorded = paths::write_file(&paths::pidfile(), &format!("{pid}\n")).and_then(|()| {
        let json = serde_json::to_string(&state).expect("state serializes");
        paths::write_file(&state_file(), &json)
    });
    if let Err(e) = recorded {
        let _ = child.kill();
        return Err(e);
    }
    drop(guard);
    say(&format!(
        "{} ({}) starting, pid {pid} — waiting for health...",
        profile.name, profile.alias
    ));
    wait_for_health(&mut child, profile)?;
    // Reap the server if it exits while this process (the tray) lives on.
    thread::spawn(move || child.wait());
    if profile.hermes {
        match hermes::update(&profile.alias, profile.context) {
            Ok(()) => say(&format!(
                "ready: {} on :{} — hermes config updated (ctx {}); restart hermes sessions",
                profile.alias, profile.port, profile.context
            )),
            Err(e) => say(&format!(
                "ready: {} on :{} — hermes config NOT updated: {e}",
                profile.alias, profile.port
            )),
        }
    } else {
        say(&format!("ready: {} on :{}", profile.alias, profile.port));
    }
    Ok(())
}

fn wait_for_health(child: &mut Child, profile: &Profile) -> Result<()> {
    let deadline = Instant::now() + seconds("S2_START_TIMEOUT", 240);
    loop {
        if !matches!(child.try_wait(), Ok(None)) {
            let _guard = lock()?;
            // Someone else may have replaced it; only clean up our own record.
            let ours = read_pid() == Some(child.id() as i32);
            if ours {
                let _ = std::fs::remove_file(paths::pidfile());
                let _ = std::fs::remove_file(state_file());
                return Err(format!(
                    "server died; see {}\n{}",
                    paths::log().display(),
                    log_tail()
                ));
            }
            return Err(format!("{} was stopped before it was ready", profile.name));
        }
        if healthy(profile.port) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out; see {}", paths::log().display()));
        }
        thread::sleep(Duration::from_secs(1).min(seconds("S2_START_TIMEOUT", 240)));
    }
}
