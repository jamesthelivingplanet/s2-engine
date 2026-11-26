//! The JSON file holding runtimes and profiles.

use crate::{Result, paths};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Runtime name to the path (or command name) of its `llama-server`.
    pub runtimes: BTreeMap<String, String>,
    /// In menu order.
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime: String,
    /// Path of the model file.
    pub model: String,
    /// Name the server reports to its clients.
    pub alias: String,
    pub context: u32,
    pub port: u16,
    /// Extra server flags, one per element.
    #[serde(default)]
    pub args: Vec<String>,
    /// Point hermes' default model at this profile when it starts.
    #[serde(default)]
    pub hermes: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Hugging Face repository.
    pub repo: String,
    /// File path inside the repository.
    pub file: String,
    /// Exact size in bytes, used to spot partial downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl Profile {
    pub fn model_path(&self) -> PathBuf {
        PathBuf::from(paths::expand(&self.model))
    }

    /// The directory `hf` must download into so the file lands at `model_path`.
    pub fn download_dir(&self) -> Option<PathBuf> {
        let source = self.source.as_ref()?;
        let model = self.model_path();
        if !model.ends_with(&source.file) {
            return None;
        }
        let mut dir = model;
        for _ in Path::new(&source.file).components() {
            dir.pop();
        }
        Some(dir)
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = serde_json::from_str(text).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    pub fn load() -> Result<Config> {
        let path = paths::config();
        let text = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!(
                "no config at {}; create one with `s2 add <name>` or `s2 edit`",
                path.display()
            ),
            _ => format!("cannot read {}: {e}", path.display()),
        })?;
        Config::parse(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))
    }

    /// An empty config for a first `add`.
    pub fn starter() -> Config {
        Config {
            runtimes: BTreeMap::from([("system".to_string(), "llama-server".to_string())]),
            profiles: Vec::new(),
        }
    }

    pub fn load_or_starter() -> Result<Config> {
        match std::fs::metadata(paths::config()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::starter()),
            _ => Config::load(),
        }
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("config serializes");
        text.push('\n');
        text
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        paths::write_file(&paths::config(), &self.to_json())
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for profile in &self.profiles {
            let name = &profile.name;
            check_name(name)?;
            if !seen.insert(name) {
                return Err(format!("two profiles are named '{name}'"));
            }
            let bad = |what: &str| Err(format!("profile '{name}': {what}"));
            if !self.runtimes.contains_key(&profile.runtime) {
                return bad(&format!("unknown runtime '{}'", profile.runtime));
            }
            if profile.model.is_empty() {
                return bad("model path is empty");
            }
            if profile.alias.is_empty() || profile.alias.contains(char::is_whitespace) {
                return bad("alias must be non-empty and contain no spaces");
            }
            if profile.context == 0 {
                return bad("context must be above 0");
            }
            if profile.port == 0 {
                return bad("port must be above 0");
            }
            if let Some(source) = &profile.source {
                let inside = Path::new(&source.file)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)));
                if source.repo.is_empty() || source.file.is_empty() || !inside {
                    return bad("source needs a repo and a relative file path");
                }
                if profile.download_dir().is_none() {
                    return bad("model path must end with the source file path");
                }
            }
        }
        Ok(())
    }

    /// Look up a profile by name or by its number in the list.
    pub fn find(&self, wanted: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .enumerate()
            .find(|(i, p)| p.name == wanted || (i + 1).to_string() == wanted)
            .map(|(_, p)| p)
            .ok_or_else(|| format!("unknown profile: {wanted}"))
    }

    pub fn runtime_of(&self, profile: &Profile) -> Result<String> {
        self.runtimes
            .get(&profile.runtime)
            .map(|path| paths::expand(path))
            .ok_or_else(|| format!("unknown runtime '{}'", profile.runtime))
    }
}

pub fn check_name(name: &str) -> Result<()> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    if name.is_empty() || !name.chars().all(allowed) || name.starts_with('-') {
        return Err(format!(
            "invalid profile name '{name}': use letters, digits, '.', '_' or '-'"
        ));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "invalid profile name '{name}': a number would clash with list positions"
        ));
    }
    Ok(())
}
