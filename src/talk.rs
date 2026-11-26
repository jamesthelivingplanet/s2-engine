//! A plain terminal chat against the running server.

use crate::Result;
use crate::cli::ask;
use crate::config::Profile;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// While a reply streams, Ctrl-C cancels the reply; at the prompt it exits.
fn catch_interrupt(catch: bool) {
    let handler = if catch {
        on_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t
    } else {
        libc::SIG_DFL
    };
    // SAFETY: the handler only stores to an atomic, which is signal safe.
    unsafe { libc::signal(libc::SIGINT, handler) };
}

enum Outcome {
    Reply(String),
    Interrupted,
}

pub fn chat(profile: &Profile, system: &str) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", profile.port);
    let mut messages: Vec<Value> = Vec::new();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    println!(
        "Connected to {url} ({}). /reset clears history, Ctrl-D exits.",
        profile.alias
    );
    while let Some(line) = ask("\n> ") {
        match line.as_str() {
            "" => continue,
            "/reset" => {
                messages.retain(|m| m["role"] == "system");
                println!("(history cleared)");
                continue;
            }
            _ => {}
        }
        messages.push(json!({"role": "user", "content": line}));
        let body = json!({"model": profile.alias, "messages": messages, "stream": true});
        match stream(&url, body.to_string()) {
            Ok(Outcome::Reply(reply)) => {
                println!();
                messages.push(json!({"role": "assistant", "content": reply}));
            }
            Ok(Outcome::Interrupted) => {
                println!("{RESET}\n(interrupted)");
                messages.pop();
            }
            Err(e) => {
                println!("{RESET}\nError: {e}");
                messages.pop();
            }
        }
    }
    println!();
    Ok(())
}

fn stream(url: &str, body: String) -> Result<Outcome> {
    let (send, receive) = mpsc::channel::<Result<String>>();
    let url = url.to_string();
    // Reading blocks, so it happens aside; dropping the receiver ends it.
    thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).build().into();
        let response = agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(body);
        match response {
            Err(e) => drop(send.send(Err(e.to_string()))),
            Ok(response) => {
                for line in BufReader::new(response.into_body().into_reader()).lines() {
                    if send.send(line.map_err(|e| e.to_string())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    INTERRUPTED.store(false, Ordering::SeqCst);
    catch_interrupt(true);
    let outcome = print_stream(&receive);
    catch_interrupt(false);
    outcome
}

fn print_stream(lines: &mpsc::Receiver<Result<String>>) -> Result<Outcome> {
    let mut out = std::io::stdout();
    let (mut reply, mut thinking) = (String::new(), false);
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Ok(Outcome::Interrupted);
        }
        let line = match lines.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => line?,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        if data.trim() == "[DONE]" {
            break;
        }
        let chunk: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
        let delta = &chunk["choices"][0]["delta"];
        if let Some(text) = delta["reasoning_content"]
            .as_str()
            .filter(|t| !t.is_empty())
        {
            if !thinking {
                print!("{DIM}");
                thinking = true;
            }
            print!("{text}");
        }
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            if thinking {
                println!("{RESET}");
                thinking = false;
            }
            print!("{text}");
            reply.push_str(text);
        }
        out.flush().map_err(|e| e.to_string())?;
    }
    if thinking {
        print!("{RESET}");
    }
    Ok(Outcome::Reply(reply))
}
