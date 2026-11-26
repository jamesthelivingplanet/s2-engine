//! Every file the tool touches, each overridable so tests stay away from the real ones.

use std::env;
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

fn var_or(name: &str, default: impl FnOnce() -> PathBuf) -> PathBuf {
    match env::var_os(name) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => default(),
    }
}

fn config_home() -> PathBuf {
    var_or("XDG_CONFIG_HOME", || home().join(".config"))
}

pub fn config() -> PathBuf {
    var_or("S2_CONFIG", || config_home().join("s2-engine/models.json"))
}

pub fn pidfile() -> PathBuf {
    var_or("S2_PIDFILE", || home().join("llama-server.pid"))
}

pub fn log() -> PathBuf {
    var_or("S2_LOG", || home().join("llama-server.log"))
}

pub fn state_dir() -> PathBuf {
    var_or("S2_STATE_DIR", || home().join(".local/state/s2-engine"))
}

pub fn hermes_config() -> PathBuf {
    var_or("S2_HERMES_CONFIG", || home().join(".hermes/config.yaml"))
}

pub fn systemd_user_dir() -> PathBuf {
    config_home().join("systemd/user")
}

/// Expand a leading `~/` the way a shell would.
pub fn expand(text: &str) -> String {
    match text.strip_prefix("~/") {
        Some(rest) => home().join(rest).to_string_lossy().into_owned(),
        None => text.to_string(),
    }
}

/// The file a write should land on: follows symlinks (even dangling ones) so a
/// linked config stays linked.
pub fn write_target(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::read_link(&current) {
            Ok(next) if next.is_absolute() => current = next,
            Ok(next) => current = current.parent().unwrap_or(Path::new(".")).join(next),
            Err(_) => break,
        }
    }
    current
}

/// Replace a file atomically, keeping its permissions and any symlink pointing at it.
pub fn write_file(path: &Path, contents: &str) -> crate::Result<()> {
    let target = write_target(path);
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", target.display());
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        std::fs::write(&temp, contents)?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temp, meta.permissions())?;
        }
        std::fs::rename(&temp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(fail)
}
