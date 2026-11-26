//! Command line parsing and the everyday commands.

use crate::config::{Config, Profile};
use crate::{Result, manage, server, skill, talk, tray};
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "s2",
    bin_name = "s2",
    version,
    about = "Run local language models, one at a time (also installed as s2-engine)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List profiles and which one is running (the default)
    List,
    /// Start a profile, replacing whatever is running
    Start {
        /// Profile name or list number; asks when omitted
        profile: Option<String>,
        /// Allow stopping a running model when not in a terminal
        #[arg(long)]
        force: bool,
        /// Print the server command instead of running it
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop the running model
    Stop {
        /// Allow stopping a running model when not in a terminal
        #[arg(long)]
        force: bool,
    },
    /// Show what is running
    Status,
    /// Chat in the terminal, starting the profile first if needed
    Talk {
        profile: Option<String>,
        /// Optional system prompt
        #[arg(trailing_var_arg = true)]
        system: Vec<String>,
    },
    /// Add a profile
    Add(manage::Add),
    /// Remove a profile
    Remove {
        name: String,
        /// Also delete the model file, if no other profile uses it
        #[arg(long)]
        delete_file: bool,
        /// Skip the confirmation for --delete-file
        #[arg(long)]
        yes: bool,
    },
    /// Print one profile
    Show { name: String },
    /// Edit the config file in your editor
    Edit,
    /// Download missing models from their sources
    Pull {
        /// Only this profile
        name: Option<String>,
        /// Report only; exit 1 if something can be downloaded
        #[arg(long)]
        check: bool,
    },
    /// Show the system tray icon, or set it up to start at login
    Tray {
        #[command(subcommand)]
        action: Option<tray::Action>,
    },
    /// Install the agent skill for Claude, Codex and Hermes
    Skill {
        #[command(subcommand)]
        action: skill::Action,
    },
}

pub fn main() -> ExitCode {
    match run(Cli::parse().command.unwrap_or(Command::List)) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn say(message: &str) {
    println!("{message}");
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// Ask a question on the terminal; None on end of input.
pub fn ask(question: &str) -> Option<String> {
    print!("{question}");
    std::io::stdout().flush().ok()?;
    let mut answer = String::new();
    match std::io::stdin().read_line(&mut answer) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(answer.trim().to_string()),
    }
}

pub fn confirm(question: &str) -> bool {
    matches!(
        ask(&format!("{question} [y/N] ")).as_deref(),
        Some("y" | "Y")
    )
}

/// An agent using the running model must not cut itself off by accident.
fn guard(config: Option<&Config>, force: bool) -> Result<()> {
    match server::running(config) {
        Some(run) if !force && !interactive() => Err(format!(
            "{} is running and this is not a terminal; stopping it needs --force \
             (agents: ask the user first)",
            run.alias
        )),
        _ => Ok(()),
    }
}

fn run(command: Command) -> Result<ExitCode> {
    // Piping into `head` should end quietly. The tray keeps Rust's default, so a
    // lost D-Bus connection reaches it as an error instead of killing it.
    if !matches!(command, Command::Tray { .. }) {
        // SAFETY: restoring the default disposition installs no handler.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    match command {
        Command::List => {
            let config = Config::load()?;
            println!("Profiles:");
            list(&config);
            println!(
                "\nUsage: s2 start [profile] | stop | status | talk <profile> | \
                 add | remove | show | edit | pull | tray | skill  (s2 --help)"
            );
        }
        Command::Start {
            profile,
            force,
            dry_run,
        } => {
            let config = Config::load()?;
            let wanted = match profile {
                Some(wanted) => wanted,
                None => {
                    println!("Select a profile:");
                    list(&config);
                    if !interactive() {
                        return Ok(ExitCode::from(2));
                    }
                    ask("> ").unwrap_or_default()
                }
            };
            let profile = config.find(&wanted)?;
            if dry_run {
                let line = server::command_line(&config, profile)?;
                let quoted: Vec<_> = line.iter().map(|word| quote(word)).collect();
                println!("{}", quoted.join(" "));
            } else {
                guard(Some(&config), force)?;
                server::start(&config, profile, &mut say)?;
            }
        }
        // Stopping and status keep working when the config is broken.
        Command::Stop { force } => {
            let config = Config::load().ok();
            guard(config.as_ref(), force)?;
            println!("{}", server::stop(config.as_ref())?);
        }
        Command::Status => status(Config::load().ok().as_ref()),
        Command::Talk { profile, system } => {
            let config = Config::load()?;
            let Some(wanted) = profile else {
                println!("usage: s2 talk <profile> [system prompt]\n");
                list(&config);
                return Ok(ExitCode::from(2));
            };
            let profile = config.find(&wanted)?;
            if !server::running_is(&config, profile) {
                if let Some(run) = server::running(Some(&config)) {
                    if !interactive() {
                        return Err(format!(
                            "{} is running, not {}; run: s2 start {} --force",
                            run.alias, profile.name, profile.name
                        ));
                    }
                    let question = format!(
                        "{} is running. Stop it and start {}?",
                        run.alias, profile.name
                    );
                    if !confirm(&question) {
                        return Ok(ExitCode::FAILURE);
                    }
                }
                server::start(&config, profile, &mut say)?;
            }
            talk::chat(profile, &system.join(" "))?;
        }
        Command::Add(add) => manage::add(add)?,
        Command::Remove {
            name,
            delete_file,
            yes,
        } => manage::remove(&name, delete_file, yes)?,
        Command::Show { name } => manage::show(&name)?,
        Command::Edit => manage::edit()?,
        Command::Pull { name, check } => {
            if !manage::pull(&Config::load()?, name.as_deref(), check)? {
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::Tray { action } => tray::run(action)?,
        Command::Skill { action } => skill::run(action)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn list(config: &Config) {
    let running = server::running(Some(config));
    let current = running.as_ref().and_then(|r| r.profile.as_deref());
    let width = |of: fn(&Profile) -> usize, least| {
        config.profiles.iter().map(of).max().unwrap_or(0).max(least)
    };
    let name = width(|p| p.name.chars().count(), 8);
    let description = width(|p| p.description.chars().count(), 0).min(60);
    for (i, p) in config.profiles.iter().enumerate() {
        let state = if p.model_path().is_file() {
            "ok"
        } else {
            "MISSING"
        };
        let mark = if current == Some(p.name.as_str()) {
            "   <- running"
        } else {
            ""
        };
        println!(
            "  {}) {:name$} {:description$} {state}{mark}",
            i + 1,
            p.name,
            p.description
        );
    }
    if config.profiles.is_empty() {
        println!("  (none yet: s2 add <name>)");
    }
    if let Some(run) = running.as_ref().filter(|r| r.profile.is_none()) {
        println!(
            "  running: {} (pid {}), not matching a profile",
            run.alias, run.pid
        );
    }
}

fn status(config: Option<&Config>) {
    match server::running(config) {
        None => println!("not running"),
        Some(run) => {
            let profile = run
                .profile
                .map(|p| format!(" [profile {p}]"))
                .unwrap_or_default();
            let ready = run.port.is_none_or(server::healthy);
            let starting = if ready { "" } else { " (starting)" };
            println!("running pid {}: {}{profile}{starting}", run.pid, run.alias);
        }
    }
}

/// Quote a word for display so it can be pasted into a shell.
fn quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./=:,+".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
