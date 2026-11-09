//! The agent row menu (A1) against the in-process fake core and a fake `gh` (`CONTR_GH`): a
//! secondary press on `agent/t17` opens a menu (`agent/t17/menu-item0..2`: Interrupt, Stop, Open PR);
//! Interrupt and Stop call the core, Open PR runs `gh pr view --web` in the agent's folder and is
//! off while `gh pr view --json number` fails. A failure shows in the `error` line.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use common::*;
use serde_json::{Value, json};

const MIN: u64 = 60_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A fake `gh` that logs `<cwd> <args>` to `<script>.log` and fails `pr view --json` unless `has_pr`.
fn fake_gh(name: &str, has_pr: bool) -> PathBuf {
    let script = std::env::temp_dir().join(format!("contr-rune-gh-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(format!("{}.log", script.display()));
    let json_exit = if has_pr { "exit 0" } else { "exit 1" };
    let body = format!("#!/bin/sh\necho \"$PWD $*\" >> \"{}.log\"\ncase \"$*\" in *--json*) {json_exit};; esac\nexit 0\n", script.display());
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn gh_log(script: &PathBuf) -> String {
    std::fs::read_to_string(format!("{}.log", script.display())).unwrap_or_default()
}

fn fixture(d: &Dirs) -> Value {
    let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
    for t in fx["terminals"].as_array_mut().unwrap() {
        t["cwd"] = json!(d.proj);
        t["status_ms"] = json!(now_ms() - 2 * MIN);
    }
    fx
}

/// Right-click `agent/t17`, then choose menu item `item`.
fn run(item: usize, gh: &PathBuf, extra: impl FnOnce(&mut Value) + 'static) -> Run {
    let events = vec![json!({"t": 0.5, "type": "secondary-click", "key": "agent/t17"}), tap(1.2, &format!("agent/t17/menu-item{item}"))];
    App::new()
        .project("contr")
        .scene("agents")
        .fixture_with(move |d| {
            let mut v = fixture(d);
            extra(&mut v);
            v
        })
        .env("CONTR_GH", gh.to_str().unwrap())
        .events(events)
        .end(2.0)
        .run()
}

#[test]
fn interrupt_calls_the_core_and_a_failure_shows_in_the_error_line() {
    let gh = fake_gh("interrupt", true);
    let r = run(0, &gh, |_| {});
    assert_eq!(r.calls("terminal.interrupt"), [&json!({"method": "terminal.interrupt", "params": {"terminal": "t17"}})]);
    assert!(!r.has("agent/t17/menu-menu"), "the menu closes after a choice");
    assert!(!r.has("error"));

    let r = run(0, &gh, |fx| fx["errors"] = json!({"terminal.interrupt": {"kind": "not_found", "message": "no such terminal"}}));
    assert!(r.find("error").is_some_and(|e| e["text"].as_str().unwrap_or("").contains("no such terminal")), "{:?}", r.find("error"));
}

#[test]
fn stop_kills_then_removes_the_terminal() {
    let gh = fake_gh("stop", true);
    let r = run(1, &gh, |_| {});
    let methods: Vec<&str> = r.log.iter().filter_map(|l| l["method"].as_str()).filter(|m| *m == "terminal.kill" || *m == "terminal.rm").collect();
    assert_eq!(methods, ["terminal.kill", "terminal.rm"]);
    assert_eq!(r.calls("terminal.kill")[0]["params"]["terminal"], "t17");
}

#[test]
fn open_pr_runs_gh_in_the_agent_folder_and_is_off_without_a_pr() {
    let gh = fake_gh("pr", true);
    let r = run(2, &gh, |_| {});
    let log = gh_log(&gh);
    assert!(log.contains("pr view --json number"), "{log}");
    let web = log.lines().find(|l| l.ends_with("pr view --web")).unwrap_or_else(|| panic!("{log}"));
    // macOS may report the scratch folder as `/private/var/...`.
    assert!(web.trim_start_matches("/private").starts_with(r.project.trim_start_matches("/private")), "{web} in {}", r.project);

    let gh = fake_gh("nopr", false);
    let r = run(2, &gh, |_| {});
    assert!(gh_log(&gh).contains("pr view --json number"));
    assert!(!gh_log(&gh).contains("--web"), "no PR: Open PR does nothing");
    assert!(!r.has("error"));
}
