//! Changing the set of profiles, and fetching their models.

use crate::cli::{ask, confirm, interactive};
use crate::config::{Config, Profile, Source, check_name};
use crate::{Result, paths, server};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(clap::Args)]
pub struct Add {
    /// Name used with `s2 start`
    name: String,
    #[arg(long)]
    description: Option<String>,
    /// Runtime name from the config
    #[arg(long)]
    runtime: Option<String>,
    /// Model file path
    #[arg(long)]
    model: Option<String>,
    /// Name the server reports (defaults to the profile name)
    #[arg(long)]
    alias: Option<String>,
    /// Context size in tokens
    #[arg(long)]
    context: Option<u32>,
    #[arg(long)]
    port: Option<u16>,
    /// Point hermes' default model here when this profile starts
    #[arg(long)]
    hermes: bool,
    /// Hugging Face repository to download from
    #[arg(long, requires = "file")]
    repo: Option<String>,
    /// File inside that repository
    #[arg(long, requires = "repo")]
    file: Option<String>,
    /// Record the source without downloading now
    #[arg(long)]
    no_download: bool,
    /// Extra server flags, after `--`
    #[arg(last = true)]
    args: Vec<String>,
}

/// Make a path independent of the current directory; `~/` paths stay readable.
fn absolute(path: &str) -> String {
    if path.starts_with("~/") || Path::new(path).is_absolute() {
        return path.to_string();
    }
    std::path::absolute(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

fn found(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|dirs| std::env::split_paths(&dirs).any(|d| d.join(program).is_file()))
}

fn prompt(question: &str, default: &str) -> Result<String> {
    let hint = if default.is_empty() {
        String::new()
    } else {
        format!(" [{default}]")
    };
    let answer = ask(&format!("{question}{hint}: ")).ok_or("cancelled")?;
    Ok(if answer.is_empty() {
        default.to_string()
    } else {
        answer
    })
}

fn prompt_number<T: std::str::FromStr>(question: &str, default: &str) -> Result<T> {
    loop {
        match prompt(question, default)?.parse() {
            Ok(number) => return Ok(number),
            Err(_) => println!("please enter a number in range"),
        }
    }
}

/// Ask for whatever required value is missing, or list them all when nobody can answer.
fn complete(add: &mut Add, config: &Config) -> Result<()> {
    let mut missing = Vec::new();
    if add.runtime.is_none() {
        missing.push("--runtime");
    }
    if add.model.is_none() && add.repo.is_none() {
        missing.push("--model (or --repo and --file)");
    }
    if add.context.is_none() {
        missing.push("--context");
    }
    if add.port.is_none() {
        missing.push("--port");
    }
    if missing.is_empty() {
        return Ok(());
    }
    if !interactive() {
        return Err(format!("missing required values: {}", missing.join(", ")));
    }
    if add.runtime.is_none() {
        let names: Vec<_> = config.runtimes.keys().cloned().collect();
        println!("Runtimes: {}", names.join(", "));
        let default = names.iter().find(|n| *n == "system").or(names.first());
        add.runtime = Some(prompt("Runtime", default.map_or("", |s| s))?);
    }
    if add.model.is_none() && add.repo.is_none() {
        let answer = prompt("Model file (blank to download from Hugging Face)", "")?;
        if answer.is_empty() {
            add.repo = Some(prompt("Hugging Face repo", "")?);
            add.file = Some(prompt("File in that repo", "")?);
        } else {
            add.model = Some(answer);
        }
    }
    if add.context.is_none() {
        add.context = Some(prompt_number("Context size", "8192")?);
    }
    if add.port.is_none() {
        add.port = Some(prompt_number("Port", "8080")?);
    }
    if add.args.is_empty() {
        let flags = prompt("Extra server flags, space separated", "")?;
        add.args = flags.split_whitespace().map(String::from).collect();
    }
    Ok(())
}

pub fn add(mut add: Add) -> Result<()> {
    let mut config = Config::load_or_starter()?;
    check_name(&add.name)?;
    if config.profiles.iter().any(|p| p.name == add.name) {
        return Err(format!("a profile named '{}' already exists", add.name));
    }
    complete(&mut add, &config)?;

    let source = match (add.repo, add.file) {
        (Some(repo), Some(file)) => Some(Source {
            repo,
            file,
            size: None,
        }),
        _ => None,
    };
    let model = match (&add.model, &source) {
        (Some(model), _) => absolute(model),
        (None, Some(source)) => format!("~/models/{}/{}", add.name, source.file),
        (None, None) => return Err("a model file or a source is required".into()),
    };
    let mut profile = Profile {
        alias: add.alias.unwrap_or_else(|| add.name.clone()),
        description: add.description.unwrap_or_default(),
        runtime: add.runtime.unwrap_or_default(),
        model,
        context: add.context.unwrap_or_default(),
        port: add.port.unwrap_or_default(),
        args: add.args,
        hermes: add.hermes,
        source,
        hooks: None,
        name: add.name,
    };
    let present = profile.model_path().is_file();
    if profile.source.is_none() && !present {
        return Err(format!(
            "model file not found: {}",
            profile.model_path().display()
        ));
    }
    if present {
        record_size(&mut profile);
    }
    let name = profile.name.clone();
    let download = profile.source.is_some() && !present && !add.no_download;
    config.profiles.push(profile);
    config.save()?;
    println!("added {name} to {}", paths::config().display());
    let runtime = config.runtime_of(config.find(&name)?)?;
    if !found(&runtime) {
        println!("warning: runtime program not found: {runtime}");
    }
    if download {
        if !pull(&config, Some(&name), false)? {
            return Err(format!("the profile was saved; retry with: s2 pull {name}"));
        }
        let profile = config.profiles.last_mut().expect("just added");
        record_size(profile);
        config.save()?;
    }
    Ok(())
}

fn record_size(profile: &mut Profile) {
    let size = std::fs::metadata(profile.model_path())
        .map(|m| m.len())
        .ok();
    if let Some(source) = &mut profile.source {
        source.size = size;
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

pub fn remove(wanted: &str, delete_file: bool, yes: bool) -> Result<()> {
    let mut config = Config::load()?;
    let profile = config.find(wanted)?.clone();
    let model = profile.model_path();
    if delete_file {
        let user = config
            .profiles
            .iter()
            .filter(|p| p.name != profile.name)
            .find(|p| {
                same_file(&p.model_path(), &model)
                    || p.args
                        .iter()
                        .any(|a| same_file(Path::new(&paths::expand(a)), &model))
            });
        if let Some(other) = user {
            return Err(format!(
                "not deleting {}: profile '{}' also uses it (remove without --delete-file)",
                model.display(),
                other.name
            ));
        }
        if server::uses(&model) {
            return Err(format!(
                "{} is in use by the running server; stop it first",
                model.display()
            ));
        }
        if !yes {
            if !interactive() {
                return Err("deleting a model file outside a terminal needs --yes".into());
            }
            let question = format!("Remove {} and delete {}?", profile.name, model.display());
            if !confirm(&question) {
                return Err("nothing changed".into());
            }
        }
    }
    config.profiles.retain(|p| p.name != profile.name);
    config.save()?;
    println!("removed profile {}", profile.name);
    if delete_file {
        match std::fs::remove_file(&model) {
            Ok(()) => println!("deleted {}", model.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("no file to delete at {}", model.display());
            }
            Err(e) => return Err(format!("cannot delete {}: {e}", model.display())),
        }
    }
    Ok(())
}

pub fn show(wanted: &str) -> Result<()> {
    let config = Config::load()?;
    let json = serde_json::to_string_pretty(config.find(wanted)?).expect("profile serializes");
    println!("{json}");
    Ok(())
}

pub fn edit() -> Result<()> {
    if !interactive() {
        return Err(format!(
            "s2 edit needs a terminal; the file is {}",
            paths::config().display()
        ));
    }
    let path = paths::config();
    let original = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let draft: PathBuf = paths::state_dir().join("models.edit.json");
    let start = original
        .clone()
        .unwrap_or_else(|| Config::starter().to_json());
    paths::write_file(&draft, &start)?;
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| {
            if found("nano") {
                "nano".into()
            } else {
                "vi".into()
            }
        });
    loop {
        // Through the shell so that an editor given with flags works.
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(&draft)
            .status()
            .map_err(|e| format!("cannot run {editor}: {e}"))?;
        if !status.success() {
            return Err("editor failed; the config is unchanged".into());
        }
        let text = std::fs::read_to_string(&draft)
            .map_err(|e| format!("cannot read {}: {e}", draft.display()))?;
        match Config::parse(&text) {
            Ok(_) => {
                if original.as_deref() == Some(text.as_str()) {
                    println!("no changes");
                } else {
                    paths::write_file(&path, &text)?;
                    println!("saved {}", path.display());
                }
                let _ = std::fs::remove_file(&draft);
                return Ok(());
            }
            Err(e) => {
                println!("not saved: {e}");
                let again = ask("Edit again? [Y/n] ");
                if !matches!(again.as_deref(), Some("" | "y" | "Y")) {
                    return Err(format!(
                        "the config is unchanged; your draft is at {}",
                        draft.display()
                    ));
                }
            }
        }
    }
}

fn complete_file(path: &Path, size: Option<u64>) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && size.is_none_or(|s| s == m.len()))
}

/// Fetch (or with `check`, report) missing models. False means something failed,
/// or with `check`, that something is waiting to be downloaded.
pub fn pull(config: &Config, wanted: Option<&str>, check: bool) -> Result<bool> {
    let profiles: Vec<&Profile> = match wanted {
        Some(wanted) => vec![config.find(wanted)?],
        None => config.profiles.iter().collect(),
    };
    let hf = std::env::var("S2_HF").unwrap_or_else(|_| "hf".into());
    let mut good = true;
    let mut seen = Vec::new();
    // Profiles with a source go first, so a shared model is fetched by the one that can.
    let ordered = profiles
        .iter()
        .filter(|p| p.source.is_some())
        .chain(profiles.iter().filter(|p| p.source.is_none()));
    for profile in ordered {
        let model = profile.model_path();
        if seen.contains(&model) {
            continue;
        }
        seen.push(model.clone());
        let size = profile.source.as_ref().and_then(|s| s.size);
        if complete_file(&model, size) {
            println!("ok       {}", model.display());
            continue;
        }
        let (Some(source), Some(dir)) = (&profile.source, profile.download_dir()) else {
            println!(
                "MISSING  {}  (no source: get this file yourself)",
                model.display()
            );
            good &= wanted.is_none();
            continue;
        };
        let megabytes = size
            .map(|s| format!("  ({} MB)", s / 1_000_000))
            .unwrap_or_default();
        println!(
            "MISSING  {}{megabytes}  from huggingface.co/{}",
            model.display(),
            source.repo
        );
        if check {
            good = false;
            continue;
        }
        let status = std::fs::create_dir_all(&dir)
            .and_then(|()| {
                Command::new(&hf)
                    .args(["download", &source.repo, &source.file, "--local-dir"])
                    .arg(&dir)
                    .status()
            })
            .map_err(|e| format!("cannot run {hf}: {e}"))?;
        if !status.success() {
            println!("download failed: {}", source.repo);
            good = false;
        } else if !complete_file(&model, size) {
            println!("size mismatch after download: {}", model.display());
            good = false;
        }
    }
    // Extra files such as adapters have no source, so only point them out.
    for profile in &profiles {
        for pair in profile.args.windows(2) {
            let file_flag =
                ["--lora", "--mmproj", "-md", "--model-draft"].contains(&pair[0].as_str());
            let path = paths::expand(&pair[1]);
            if file_flag && !Path::new(&path).is_file() {
                println!("warning: {} needs {path}, which is missing", profile.name);
            }
        }
    }
    Ok(good)
}
