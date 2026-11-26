//! The agent skill, shipped inside the program so it always matches it.

use crate::{Result, paths};
use std::path::{Path, PathBuf};

const SKILL: &str = include_str!("../skill/SKILL.md");
const NAME: &str = "s2-engine";

#[derive(clap::Subcommand)]
pub enum Action {
    /// Write the skill and link it into each installed agent
    Install,
    /// Undo `install`
    Remove,
}

pub fn run(action: Action) -> Result<()> {
    match action {
        Action::Install => install(),
        Action::Remove => remove(),
    }
}

/// The one real copy; Claude's skills folder links here already.
fn copy_dir() -> PathBuf {
    paths::home().join(".agents/skills").join(NAME)
}

/// Agents with their own skills folder.
fn agent_dirs() -> [PathBuf; 3] {
    let home = paths::home();
    [".claude/skills", ".codex/skills", ".hermes/skills"].map(|d| home.join(d))
}

fn same(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn points_here(link: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|target| target == copy_dir())
}

fn install() -> Result<()> {
    let dir = copy_dir();
    let file = dir.join("SKILL.md");
    paths::write_file(&file, SKILL)?;
    println!("wrote {}", file.display());
    for skills in agent_dirs().iter().filter(|d| d.is_dir()) {
        let link = skills.join(NAME);
        if points_here(&link) {
            println!("linked {}", link.display());
        } else if same(&link, &dir) {
            // The agent's folder is itself a link to the shared one.
            println!("shared {}", link.display());
        } else if link.symlink_metadata().is_ok() {
            println!(
                "skipped {}: something else is already there",
                link.display()
            );
        } else {
            std::os::unix::fs::symlink(&dir, &link)
                .map_err(|e| format!("cannot link {}: {e}", link.display()))?;
            println!("linked {}", link.display());
        }
    }
    Ok(())
}

fn remove() -> Result<()> {
    for link in agent_dirs().iter().map(|d| d.join(NAME)) {
        if points_here(&link) {
            std::fs::remove_file(&link)
                .map_err(|e| format!("cannot remove {}: {e}", link.display()))?;
            println!("removed {}", link.display());
        }
    }
    let dir = copy_dir();
    let file = dir.join("SKILL.md");
    if file.exists() {
        std::fs::remove_file(&file)
            .map_err(|e| format!("cannot remove {}: {e}", file.display()))?;
        println!("removed {}", file.display());
    }
    // Only an empty folder goes; anything added by hand stays.
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}
