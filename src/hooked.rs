//! Profiles with hooks run inside a transient systemd unit.
//!
//! systemd runs `pre`, then `s2 __serve` as the user, then `post` however that
//! ends: stop, crash, failed start or failed `pre`. The unit lives outside the
//! login session, so it survives a `pre` that closes the desktop.

use crate::config::{Config, Hooks, Profile};
use crate::{Result, paths, server};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The user's own systemd, for hooks that need no root.
    User,
    /// The system's, for `sudo` hooks.
    System,
}

pub fn unit_name() -> String {
    std::env::var("S2_UNIT").unwrap_or_else(|_| "s2-hooked".into())
}

fn systemctl(scope: Scope) -> Command {
    let program = std::env::var("S2_SYSTEMCTL").unwrap_or_else(|_| "systemctl".into());
    let mut command = Command::new(program);
    if scope == Scope::User {
        command.arg("--user");
    }
    command
}

fn busy(scope: Scope) -> bool {
    let output = systemctl(scope)
        .args(["show", "--property=ActiveState", "--value"])
        .arg(format!("{}.service", unit_name()))
        .output();
    output.is_ok_and(|o| {
        let state = String::from_utf8_lossy(&o.stdout);
        ["active", "activating", "deactivating", "reloading"].contains(&state.trim())
    })
}

/// The scope in which the hooked unit is doing something, if any.
pub fn active() -> Option<Scope> {
    [Scope::User, Scope::System].into_iter().find(|s| busy(*s))
}

pub fn wait_until_gone(scope: Scope) -> Result<()> {
    let deadline = Instant::now() + server::seconds("S2_HOOK_TIMEOUT", 180);
    while busy(scope) {
        if Instant::now() >= deadline {
            return Err(format!(
                "the post command has not finished; see: {}",
                journal(scope)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Stop a unit that has no server yet (or any more).
pub fn interrupt(scope: Scope) -> Result<String> {
    if scope == Scope::System {
        return Err(format!(
            "{0} is running a hook; to stop it: sudo systemctl stop {0}",
            unit_name()
        ));
    }
    let status = systemctl(scope)
        .arg("stop")
        .arg(format!("{}.service", unit_name()))
        .status()
        .map_err(|e| format!("cannot run systemctl: {e}"))?;
    if !status.success() {
        return Err("could not stop the hooked unit".into());
    }
    Ok(format!("{} stopped", unit_name()))
}

fn journal(scope: Scope) -> String {
    let user = if scope == Scope::User { "--user " } else { "" };
    format!("journalctl {user}-u {} -n 30", unit_name())
}

fn scope_of(hooks: &Hooks) -> Scope {
    if hooks.sudo {
        Scope::System
    } else {
        Scope::User
    }
}

/// The command that launches a hooked profile: program first.
///
/// The hook text travels as an environment variable and `:` stops systemd from
/// expanding anything, so the command that runs is the one that was shown.
pub fn command(profile: &Profile) -> Result<Vec<String>> {
    let hooks = profile.hooks.as_ref().ok_or("profile has no hooks")?;
    let scope = scope_of(hooks);
    let myself = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let run = std::env::var("S2_SYSTEMD_RUN").unwrap_or_else(|_| "systemd-run".into());
    let mut line: Vec<String> = match scope {
        // -k: ask for the password every time, even if sudo remembers one.
        Scope::System => vec!["sudo".into(), "-k".into(), run],
        Scope::User => vec![run, "--user".into()],
    };
    line.extend([
        format!("--unit={}", unit_name()),
        "--collect".into(),
        "--quiet".into(),
    ]);
    let timeout = server::seconds("S2_HOOK_TIMEOUT", 180).as_secs();
    let mut property = |text: String| line.extend(["-p".into(), text]);
    property(format!("TimeoutStartSec={timeout}"));
    property(format!("TimeoutStopSec={timeout}"));
    // `+` runs a hook as root while the server itself runs as the user.
    let (prefix, path) = match scope {
        Scope::System => (
            "+:",
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin; export PATH; ",
        ),
        Scope::User => (":", ""),
    };
    if scope == Scope::System {
        // SAFETY: getuid cannot fail and takes nothing.
        property(format!("User={}", unsafe { libc::getuid() }));
    }
    for (key, variable, hook) in [
        ("ExecStartPre", "S2_PRE", &hooks.pre),
        ("ExecStopPost", "S2_POST", &hooks.post),
    ] {
        if hook.is_some() {
            property(format!(
                "{key}={prefix}/bin/sh -c '{path}eval \"${variable}\"'"
            ));
        }
    }
    for (variable, hook) in [("S2_PRE", &hooks.pre), ("S2_POST", &hooks.post)] {
        if let Some(hook) = hook {
            line.push(format!("--setenv={variable}={hook}"));
        }
    }
    // The unit starts with a bare environment, so hand over any overrides.
    let mut overrides: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| {
            let ours = name.starts_with("S2_") && name != "S2_PRE" && name != "S2_POST";
            ours || name == "XDG_CONFIG_HOME" || (scope == Scope::User && name == "HOME")
        })
        .collect();
    overrides.sort();
    line.extend(overrides.iter().map(|(k, v)| format!("--setenv={k}={v}")));
    line.extend([
        "--".into(),
        myself.to_string_lossy().into_owned(),
        "__serve".into(),
        profile.name.clone(),
    ]);
    if scope == Scope::System {
        line.push("--system".into());
    }
    Ok(line)
}

/// Replace whatever is running, hand the profile to systemd, and wait until it answers.
pub fn start(config: &Config, profile: &Profile, say: &mut dyn FnMut(&str)) -> Result<()> {
    let hooks = profile.hooks.as_ref().ok_or("profile has no hooks")?;
    let scope = scope_of(hooks);
    // Anything that can be known to fail must fail before the desktop closes.
    server::command_line(config, profile)?;
    server::clear_the_way(config, profile, say)?;
    let line = command(profile)?;
    let status = Command::new(&line[0])
        .args(&line[1..])
        .status()
        .map_err(|e| format!("cannot run {}: {e}", line[0]))?;
    let failed = |what: &str| {
        Err(format!(
            "{what}\nsee: {}\nand {}\n{}",
            journal(scope),
            paths::log().display(),
            server::log_tail()
        ))
    };
    if !status.success() {
        return failed("could not start: the pre command or systemd-run failed");
    }
    say(&format!(
        "{} handed to {}; waiting for health...",
        profile.name,
        unit_name()
    ));
    let deadline = Instant::now() + server::seconds("S2_START_TIMEOUT", 240);
    loop {
        if let Some(line) = server::ready_line(&profile.name) {
            say(&line);
            return Ok(());
        }
        if !busy(scope) {
            return failed("server died or never started");
        }
        if Instant::now() >= deadline {
            return failed("timed out");
        }
        thread::sleep(Duration::from_millis(500));
    }
}
