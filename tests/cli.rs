//! Drives the real binary against a fake server, with every file in a temp directory.

use serde_json::{Value, json};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-llama-server.py");
const FAKE_HF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-hf.sh");

struct Sandbox {
    dir: tempfile::TempDir,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn file(&self, name: &str, contents: &str) -> String {
        let path = self.path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    fn profile(&self, name: &str, extra: &[&str]) -> Value {
        let model = self.file(&format!("{name}.gguf"), "weights");
        json!({"name": name, "description": format!("the {name} profile"), "runtime": "fake",
               "model": model, "alias": format!("{name}-alias"), "context": 512,
               "port": free_port(), "args": extra})
    }

    fn config(&self, profiles: Vec<Value>) {
        let config = json!({"runtimes": {"fake": FAKE}, "profiles": profiles});
        self.file("models.json", &config.to_string());
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_s2"));
        command
            .args(args)
            .env("HOME", self.path("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env("S2_CONFIG", self.path("models.json"))
            .env("S2_PIDFILE", self.path("server.pid"))
            .env("S2_LOG", self.path("server.log"))
            .env("S2_STATE_DIR", self.path("state"))
            .env("S2_HERMES_CONFIG", self.path("hermes.yaml"))
            .env("S2_START_TIMEOUT", "15")
            .env("S2_HF", FAKE_HF)
            .env("S2_SYSTEMCTL", "true");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).stdin(Stdio::null()).output().unwrap()
    }

    /// Run, expect success, return stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.status.success(), "{args:?} failed: {}", text(&out));
        String::from_utf8(out.stdout).unwrap()
    }

    /// Run, expect failure, return everything it said.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            !out.status.success(),
            "{args:?} should fail: {}",
            text(&out)
        );
        text(&out)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["stop", "--force"]);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn alive(pid: &str) -> bool {
    std::fs::read(format!("/proc/{}/cmdline", pid.trim())).is_ok_and(|c| !c.is_empty())
}

#[test]
fn start_status_list_stop() {
    let s = Sandbox::new();
    let (mut a, b) = (s.profile("a", &[]), s.profile("b", &[]));
    // Two profiles sharing an alias must not both look like they are running.
    a["alias"] = b["alias"].clone();
    s.config(vec![a, b]);
    assert_eq!(s.ok(&["status"]), "not running\n");

    let out = s.ok(&["start", "2"]);
    assert!(out.contains("ready: b-alias on :"), "{out}");
    let pid = s.read("server.pid");
    assert!(alive(&pid), "server outlives s2");
    assert!(s.ok(&["status"]).contains("b-alias [profile b]"));
    let list = s.ok(&["list"]);
    let marked: Vec<_> = list.lines().filter(|l| l.contains("<- running")).collect();
    assert_eq!(marked.len(), 1, "{list}");
    assert!(marked[0].contains("the b profile"));
    assert_eq!(s.ok(&[]), list, "list is the default");

    assert!(s.ok(&["stop", "--force"]).contains("b-alias stopped"));
    assert!(!alive(&pid));
    assert_eq!(s.ok(&["stop"]), "no model running\n");
}

#[test]
fn replacing_a_running_model_outside_a_terminal_needs_force() {
    let s = Sandbox::new();
    s.config(vec![s.profile("a", &[]), s.profile("b", &[])]);
    s.ok(&["start", "a"]);
    let first = s.read("server.pid");
    for args in [&["start", "b"][..], &["start", "a"], &["stop"]] {
        assert!(s.fails(args).contains("--force"), "{args:?}");
    }
    assert!(
        s.fails(&["talk", "b"])
            .contains("a-alias is running, not b")
    );
    assert!(alive(&first));
    assert!(s.ok(&["start", "b", "--force"]).contains("a-alias stopped"));
    assert!(!alive(&first));
}

#[test]
fn a_server_started_by_the_old_script_is_recognised() {
    let s = Sandbox::new();
    s.config(vec![s.profile("a", &[])]);
    s.ok(&["start", "a"]);
    std::fs::remove_file(s.path("state/running.json")).unwrap();
    assert!(s.ok(&["status"]).contains("[profile a]"));
}

#[test]
fn failed_starts_report_and_clean_up() {
    let s = Sandbox::new();
    s.config(vec![
        s.profile("dies", &["--die"]),
        s.profile("hangs", &["--hang"]),
    ]);
    let out = s.fails(&["start", "dies"]);
    assert!(
        out.contains("server died") && out.contains("fake error"),
        "{out}"
    );
    assert!(!s.path("server.pid").exists());

    let out = s
        .command(&["start", "hangs"])
        .env("S2_START_TIMEOUT", "2")
        .output()
        .unwrap();
    assert!(text(&out).contains("timed out"));
    // As before, a slow server is left running.
    assert!(s.ok(&["status"]).contains("(starting)"));
}

#[test]
fn a_busy_port_is_refused() {
    let s = Sandbox::new();
    let busy = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut a = s.profile("a", &[]);
    a["port"] = busy.local_addr().unwrap().port().into();
    s.config(vec![a]);
    assert!(s.fails(&["start", "a"]).contains("already in use"));
}

#[test]
fn hermes_changes_only_for_profiles_that_ask() {
    let yaml = "model:\n  default: old\n  base_url: x\n  context_length: 1\n\nother:\n  default: keep\n  context_length: 7\n";
    let s = Sandbox::new();
    s.file("hermes.yaml", yaml);
    let (a, mut b) = (s.profile("a", &[]), s.profile("b", &[]));
    b["hermes"] = true.into();
    s.config(vec![a, b]);
    s.ok(&["start", "a"]);
    assert_eq!(s.read("hermes.yaml"), yaml);
    assert!(
        s.ok(&["start", "b", "--force"])
            .contains("hermes config updated (ctx 512)")
    );
    let expected = yaml
        .replace("default: old", "default: b-alias")
        .replace("length: 1\n", "length: 512\n");
    assert_eq!(s.read("hermes.yaml"), expected);
}

#[test]
fn talk_streams_and_keeps_history() {
    let s = Sandbox::new();
    s.config(vec![s.profile("a", &[])]);
    let mut child = s
        .command(&["talk", "a", "be", "brief"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"one\ntwo\n/reset\nthree\n")
        .unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    // system + user = 2, then 4, then back to 2 after the reset.
    let turns: Vec<_> = out
        .match_indices("turns=")
        .map(|(i, _)| &out[i..i + 7])
        .collect();
    assert_eq!(turns, ["turns=2", "turns=4", "turns=2"], "{out}");
    assert!(out.contains("(history cleared)") && out.contains("hmm"));
}

#[test]
fn dry_run_expands_home_and_keeps_order() {
    let s = Sandbox::new();
    let mut a = s.profile("a", &["--lora", "~/x.gguf", "--note", "two words"]);
    a["model"] = "~/m.gguf".into();
    s.config(vec![a]);
    let line = s.ok(&["start", "a", "--dry-run"]);
    let home = s.path("home").display().to_string();
    assert!(line.contains(&format!("-m {home}/m.gguf -c 512 --lora {home}/x.gguf --note 'two words' --alias a-alias --host 127.0.0.1 --port ")), "{line}");
    assert!(!s.path("server.pid").exists());
}

#[test]
fn add_validates_and_saves() {
    let s = Sandbox::new();
    let missing = s.fails(&["add", "a"]);
    for flag in ["--runtime", "--model", "--context", "--port"] {
        assert!(missing.contains(flag), "{missing}");
    }
    let model = s.file("m.gguf", "w");
    let add = [
        "add",
        "a",
        "--runtime",
        "system",
        "--model",
        &model,
        "--context",
        "64",
        "--port",
        "9000",
        "--",
        "-ngl",
        "99",
    ];
    s.ok(&add);
    let shown: Value = serde_json::from_str(&s.ok(&["show", "a"])).unwrap();
    assert_eq!(shown["alias"], "a");
    assert_eq!(shown["args"], json!(["-ngl", "99"]));
    assert!(s.fails(&add).contains("already exists"));
    let base = ["--context", "64", "--port", "9000"];
    for (name, runtime, model) in [
        ("b", "nope", model.as_str()),
        ("c", "system", "/no/such"),
        ("7", "system", model.as_str()),
        ("d e", "system", model.as_str()),
    ] {
        let mut args = vec!["add", name, "--runtime", runtime, "--model", model];
        args.extend(base);
        s.fails(&args);
    }
    assert_eq!(s.ok(&["list"]).matches(" ok").count(), 1);
}

#[test]
fn add_with_a_source_downloads_and_records_the_size() {
    let s = Sandbox::new();
    s.ok(&[
        "add",
        "h",
        "--runtime",
        "system",
        "--repo",
        "org/repo",
        "--file",
        "sub/m.gguf",
        "--context",
        "64",
        "--port",
        "9000",
    ]);
    assert_eq!(s.read("home/models/h/sub/m.gguf"), "12345");
    let shown: Value = serde_json::from_str(&s.ok(&["show", "h"])).unwrap();
    assert_eq!(shown["source"]["size"], 5);
    assert_eq!(shown["model"], "~/models/h/sub/m.gguf");
}

#[test]
fn pull_checks_and_fetches() {
    let s = Sandbox::new();
    let mut a = s.profile("a", &["--lora", "/no/lora.gguf"]);
    a["source"] = json!({"repo": "org/repo", "file": "a.gguf", "size": 5});
    s.config(vec![a]);
    // The file exists with the wrong size, so it counts as a partial download.
    let check = s.run(&["pull", "--check"]);
    assert_eq!(check.status.code(), Some(1));
    assert!(text(&check).contains("MISSING"));
    assert_eq!(s.read("a.gguf"), "weights");
    let out = s.ok(&["pull"]);
    assert!(out.contains("needs /no/lora.gguf"), "{out}");
    assert_eq!(s.read("a.gguf"), "12345");
    assert!(s.ok(&["pull", "--check"]).contains("ok "));
}

#[test]
fn remove_protects_shared_files_and_asks_before_deleting() {
    let s = Sandbox::new();
    let a = s.profile("a", &[]);
    let mut b = s.profile("b", &[]);
    b["model"] = a["model"].clone();
    s.config(vec![a, b]);
    assert!(
        s.fails(&["remove", "a", "--delete-file", "--yes"])
            .contains("also uses it")
    );
    s.ok(&["remove", "b"]);
    assert!(s.path("a.gguf").exists());
    assert!(s.fails(&["remove", "a", "--delete-file"]).contains("--yes"));
    s.ok(&["start", "a"]);
    assert!(
        s.fails(&["remove", "a", "--delete-file", "--yes"])
            .contains("stop it first")
    );
    s.ok(&["stop", "--force"]);
    s.ok(&["remove", "a", "--delete-file", "--yes"]);
    assert!(!s.path("a.gguf").exists());
}

#[test]
fn saving_writes_through_a_symlink() {
    let s = Sandbox::new();
    s.config(vec![s.profile("a", &[]), s.profile("b", &[])]);
    std::fs::rename(s.path("models.json"), s.path("real.json")).unwrap();
    std::os::unix::fs::symlink("real.json", s.path("models.json")).unwrap();
    s.ok(&["remove", "b"]);
    assert!(s.path("models.json").is_symlink());
    assert!(!s.read("real.json").contains("\"b\""));
}

#[test]
fn a_broken_config_is_explained_and_stop_still_works() {
    let s = Sandbox::new();
    s.config(vec![s.profile("a", &[])]);
    s.ok(&["start", "a"]);
    s.file("models.json", "{ not json");
    assert!(s.fails(&["list"]).contains("invalid config"));
    assert!(s.ok(&["status"]).contains("a-alias"));
    assert!(s.ok(&["stop", "--force"]).contains("stopped"));
    s.file("models.json", r#"{"runtimes":{},"profiles":[],"typo":1}"#);
    assert!(s.fails(&["list"]).contains("typo"));
}

#[test]
fn edit_needs_a_terminal() {
    assert!(Sandbox::new().fails(&["edit"]).contains("needs a terminal"));
}

#[test]
fn skill_installs_into_present_agents_only_and_removes_cleanly() {
    let s = Sandbox::new();
    let home = s.path("home");
    std::fs::create_dir_all(home.join(".agents/skills")).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::os::unix::fs::symlink(home.join(".agents/skills"), home.join(".claude/skills")).unwrap();
    std::fs::create_dir_all(home.join(".codex/skills")).unwrap();
    s.ok(&["skill", "install"]);
    s.ok(&["skill", "install"]);
    let reads = |p: &str| Path::new(&home).join(p).join("s2-engine/SKILL.md").exists();
    assert!(reads(".agents/skills") && reads(".claude/skills") && reads(".codex/skills"));
    assert!(!home.join(".hermes").exists());
    s.ok(&["skill", "remove"]);
    assert!(!home.join(".agents/skills/s2-engine").exists());
    assert!(
        home.join(".codex/skills/s2-engine")
            .symlink_metadata()
            .is_err()
    );
    assert!(home.join(".claude/skills").is_symlink());
}

#[test]
fn tray_setup_writes_a_unit_that_spares_the_server() {
    let s = Sandbox::new();
    s.ok(&["tray", "setup"]);
    let unit = s.read("home/.config/systemd/user/s2-tray.service");
    assert!(
        unit.contains("KillMode=process") && unit.contains("\" tray\n"),
        "{unit}"
    );
    s.ok(&["tray", "remove"]);
    assert!(!s.path("home/.config/systemd/user/s2-tray.service").exists());
}

#[test]
fn the_example_config_is_valid() {
    let example = include_str!("../examples/models.example.json");
    s2_engine::config::Config::parse(example).unwrap();
}
