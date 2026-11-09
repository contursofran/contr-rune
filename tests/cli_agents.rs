//! `contr-rune agents spawn` and `ls` against the fake core (`CONTR_HC_FAKE`): no window, no
//! real hc.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-agents-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Runs `contr-rune agents <args>` on the fake core of `fixture`; returns the exit code, stdout,
/// stderr and the log of mutating calls.
fn agents(fixture: &Path, dir: &Path, terminal_id: Option<&str>, args: &[&str]) -> (i32, String, String, Vec<Value>) {
    let log = dir.join("fake.log");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.arg("agents").args(args).current_dir(dir);
    cmd.env("CONTR_HC_FAKE", fixture).env("CONTR_HC_FAKE_LOG", &log);
    cmd.env("CONTR_AGENTS_CONFIG", dir.join("agents.json")).env("CONTR_KIT", dir.join("kit"));
    match terminal_id {
        Some(id) => cmd.env("HC_TERMINAL_ID", id),
        None => cmd.env_remove("HC_TERMINAL_ID"),
    };
    let run = cmd.output().unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    (run.status.code().unwrap(), String::from_utf8_lossy(&run.stdout).into_owned(), String::from_utf8_lossy(&run.stderr).into_owned(), calls)
}

#[test]
fn spawn_sends_the_k3_params_and_prints_the_name_and_address() {
    let dir = scratch("spawn");
    std::fs::create_dir_all(dir.join("kit/agents")).unwrap();
    std::fs::write(dir.join("kit/agents/builder.md"), "").unwrap();
    std::fs::create_dir_all(dir.join("wt")).unwrap();
    let fixture = dir.join("fixture.json");
    std::fs::write(&fixture, "{}").unwrap();

    let (code, out, err, calls) = agents(&fixture, &dir, Some("t3"), &["spawn", "--name", "b-1", "--profile", "builder", "--issue", "7", "--worktree", "wt", "fix", "it"]);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "b-1 t1\n", ""));
    let kit = dir.join("kit").display().to_string();
    let wt = dir.join("wt").display().to_string();
    assert_eq!(
        calls,
        [json!({"method": "terminal.run", "params": {
            "argv": ["claude", "--plugin-dir", kit, "--agent", "contr:builder", "--model", "sonnet", "--effort", "medium", "--permission-mode", "auto", "fix it"],
            "name": "b-1",
            "labels": {"contr": "agent", "role": "builder", "profile": "builder", "worktree": wt, "issue": "7"},
            "wait": false, "outlive_parent": true, "cwd": wt,
            "env": {"CONTR_COORDINATOR": "t3"}, "parent": "t3"
        }})]
    );

    // `--no-parent --json`, and an unknown profile is an error that names the file.
    let (code, out, _, calls) = agents(&fixture, &dir, Some("t3"), &["spawn", "--name", "b-2", "--profile", "builder", "--no-parent", "--json"]);
    assert_eq!(code, 0);
    let worker: Value = serde_json::from_str(&out).unwrap();
    assert_eq!((worker["name"].as_str(), worker["live"].as_bool()), (Some("b-2"), Some(true)));
    assert_eq!((calls[1]["params"]["no_parent"].as_bool(), calls[1]["params"].get("parent")), (Some(true), None));
    let (code, _, err, _) = agents(&fixture, &dir, None, &["spawn", "--name", "x", "--profile", "nobody"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("contr-rune agents spawn: agents: no profile \"nobody\" in "), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ls_prints_the_table_sorted_by_name_and_json() {
    let dir = scratch("ls");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc/agents.json");
    let (code, out, err, calls) = agents(&fixture, &dir, None, &["ls"]);
    assert_eq!((code, err.as_str(), calls.len()), (0, "", 0));
    let names: Vec<&str> = out.lines().skip(1).map(|l| l.split_whitespace().next().unwrap()).collect();
    assert_eq!(names, ["b-241", "b-242", "orch", "term-00001"]);
    assert!(out.starts_with("NAME        ADDRESS  ROLE          ISSUE  STATE"), "{out}");
    assert!(out.contains("blocked: permission"));

    let (code, out, _, _) = agents(&fixture, &dir, None, &["ls", "--json"]);
    assert_eq!(code, 0);
    let list: Vec<Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list.len(), 4);
    assert_eq!((list[0]["name"].as_str(), list[0]["status"].as_str(), list[0]["model"].as_str()), (Some("b-241"), Some("working"), Some("sonnet")));
    std::fs::remove_dir_all(dir).unwrap();
}
