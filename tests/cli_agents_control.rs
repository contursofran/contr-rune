//! `contr-rune agents send | interrupt | stop | rm | read | prompt` and `spawn --review-pr`
//! against the fake core (`CONTR_HC_FAKE`) and a fake `gh` (`CONTR_GH`): no window, no real hc,
//! no real worktrees.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-control-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

struct Out {
    code: i32,
    out: String,
    err: String,
    calls: Vec<Value>,
}

/// Runs `contr-rune agents <args>` on the fake core of `fixture` as the terminal `caller`.
fn agents(fixture: &Path, dir: &Path, caller: Option<&str>, envs: &[(&str, &Path)], args: &[&str]) -> Out {
    let log = dir.join("fake.log");
    let _ = std::fs::remove_file(&log);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.arg("agents").args(args).current_dir(dir);
    cmd.env("CONTR_HC_FAKE", fixture).env("CONTR_HC_FAKE_LOG", &log).env("CONTR_AGENTS_CONFIG", dir.join("agents.json"));
    cmd.envs(envs.iter().copied());
    match caller {
        Some(id) => cmd.env("HC_TERMINAL_ID", id),
        None => cmd.env_remove("HC_TERMINAL_ID"),
    };
    let run = cmd.output().unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    Out { code: run.status.code().unwrap(), out: String::from_utf8_lossy(&run.stdout).into_owned(), err: String::from_utf8_lossy(&run.stderr).into_owned(), calls }
}

const MENU: &str = "● Running\n╭────────────────╮\n│ Bash command   │\n│   cargo test   │\n│ Proceed?       │\n│ ❯ 1. Yes       │\n│   2. No        │\n╰────────────────╯\n";

fn terminal(address: &str, name: &str, role: &str, parent: Option<&str>, state: &str, status: &str) -> Value {
    json!({"address": address, "name": name, "parent": parent, "argv": ["claude"],
        "labels": {"contr": "agent", "role": role}, "state": state, "turn": 1,
        "foreground": {"pid": 1, "name": "claude", "argv": [], "cwd": "/tmp"},
        "status": {"state": status, "kind": if status == "blocked" { json!("permission") } else { Value::Null }, "msg": "", "title": "", "app": "claude-code", "progress": null}})
}

fn fixture(dir: &Path) -> PathBuf {
    let path = dir.join("fixture.json");
    let terminals = [
        terminal("t1", "b-live", "builder", Some("t3"), "live", "working"),
        terminal("t2", "b-done", "builder", Some("t3"), "finished", "idle"),
        terminal("t3", "orch", "orchestrator", None, "live", "idle"),
        terminal("t4", "b-other", "builder", Some("t9"), "finished", "idle"),
        terminal("t5", "o-done", "orchestrator", Some("t3"), "finished", "idle"),
        terminal("t6", "b-block", "builder", Some("t3"), "live", "blocked"),
        terminal("t7", "b-done2", "builder", Some("t3"), "finished", "idle"),
    ];
    let screens = json!({"t1": "line one\nline two\n\n", "t6": MENU});
    std::fs::write(&path, json!({"terminals": terminals, "screens": screens}).to_string()).unwrap();
    path
}

fn methods(o: &Out) -> Vec<(&str, &str)> {
    o.calls.iter().map(|c| (c["method"].as_str().unwrap(), c["params"]["terminal"].as_str().unwrap_or(""))).collect()
}

#[test]
fn send_types_the_text_with_the_caller_and_refuses_blank_text() {
    let dir = scratch("send");
    let fx = fixture(&dir);
    let o = agents(&fx, &dir, Some("t3"), &[], &["send", "t1", "fix", "the", "bug"]);
    assert_eq!((o.code, o.out.as_str(), o.err.as_str()), (0, "", ""));
    assert_eq!(o.calls, [json!({"method": "terminal.send", "params": {"terminal": "t1", "text": "fix the bug", "caller": "t3"}})]);

    // Blank text never reaches the core; a send the core refuses reports its message.
    let o = agents(&fx, &dir, None, &[], &["send", "t1", " ", ""]);
    assert_eq!((o.code, o.err.as_str(), o.calls.len()), (1, "contr-rune agents send: agents: no text to send\n", 0));
    let o = agents(&fx, &dir, None, &[], &["send", "t2", "hi"]);
    assert_eq!(o.code, 1);
    assert!(o.err.starts_with("contr-rune agents send: ") && o.err.contains("t2"), "{}", o.err);

    let o = agents(&fx, &dir, None, &[], &["send", "t1"]);
    assert_eq!(o.code, 2);
    assert!(o.err.starts_with("contr-rune agents send: wrong number of arguments\n"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn interrupt_prints_the_outcome_and_stop_kills_then_removes() {
    let dir = scratch("stop");
    let fx = fixture(&dir);
    let o = agents(&fx, &dir, Some("t3"), &[], &["interrupt", "t1"]);
    assert_eq!((o.code, o.out.as_str()), (0, "stopped\n"));
    assert_eq!(o.calls, [json!({"method": "terminal.interrupt", "params": {"terminal": "t1", "caller": "t3"}})]);

    let o = agents(&fx, &dir, Some("t3"), &[], &["stop", "t1"]);
    assert_eq!((o.code, methods(&o)), (0, vec![("terminal.kill", "t1"), ("terminal.rm", "t1")]));
    assert_eq!(o.calls[0]["params"]["caller"], "t3");
    // --keep leaves the finished terminal; one that already finished is not killed again.
    let o = agents(&fx, &dir, None, &[], &["stop", "--keep", "t1"]);
    assert_eq!((o.code, methods(&o)), (0, vec![("terminal.kill", "t1")]));
    let o = agents(&fx, &dir, None, &[], &["stop", "t2"]);
    assert_eq!((o.code, methods(&o)), (0, vec![("terminal.rm", "t2")]));
    let o = agents(&fx, &dir, None, &[], &["stop", "--keep", "t2"]);
    assert_eq!((o.code, o.calls.len()), (0, 0));
    // An unknown worker is an error and nothing is killed.
    let o = agents(&fx, &dir, None, &[], &["stop", "t99"]);
    assert_eq!((o.code, o.calls.len()), (1, 0));
    assert!(o.err.starts_with("contr-rune agents stop: "), "{}", o.err);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rm_removes_one_finished_worker_or_the_ended_ones_this_terminal_started() {
    let dir = scratch("rm");
    let fx = fixture(&dir);
    let o = agents(&fx, &dir, None, &[], &["rm", "t2"]);
    assert_eq!((o.code, methods(&o)), (0, vec![("terminal.rm", "t2")]));
    // A live terminal is refused by the core.
    let o = agents(&fx, &dir, None, &[], &["rm", "t1"]);
    assert_eq!(o.code, 1);

    // Only finished, non-orchestrator workers whose parent is the caller.
    let o = agents(&fx, &dir, Some("t3"), &[], &["rm", "--ended"]);
    assert_eq!((o.code, o.out.as_str()), (0, "b-done\nb-done2\n"));
    assert_eq!(methods(&o), [("terminal.rm", "t2"), ("terminal.rm", "t7")]);
    // Outside hc nothing goes.
    let o = agents(&fx, &dir, None, &[], &["rm", "--ended"]);
    assert_eq!((o.code, o.out.as_str(), o.calls.len()), (0, "", 0));

    for args in [&["rm"][..], &["rm", "--ended", "t2"], &["rm", "a", "b"]] {
        let o = agents(&fx, &dir, Some("t3"), &[], args);
        assert_eq!(o.code, 2, "{args:?}");
        assert!(o.err.starts_with("contr-rune agents rm: give one name, or --ended\n"));
    }
    let o = agents(&fx, &dir, None, &[], &["stop", "--ended", "t1"]);
    assert_eq!((o.code, o.err.starts_with("contr-rune agents stop: flag --ended does not apply\n")), (2, true));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_prints_the_screen_and_prompt_the_menu_of_a_blocked_worker() {
    let dir = scratch("read");
    let fx = fixture(&dir);
    let o = agents(&fx, &dir, Some("t3"), &[], &["read", "--lines", "20", "t1"]);
    assert_eq!((o.code, o.out.as_str()), (0, "line one\nline two\n"));
    assert_eq!(o.calls, [json!({"method": "terminal.read", "params": {"terminal": "t1", "lines": 20, "caller": "t3"}})]);
    let o = agents(&fx, &dir, None, &[], &["read", "t1", "--lines=x"]);
    assert_eq!(o.code, 2);
    assert!(o.err.starts_with("contr-rune agents read: invalid value \"x\" for flag -lines: parse error\n"));

    let o = agents(&fx, &dir, None, &[], &["prompt", "t6"]);
    assert_eq!((o.code, o.out.as_str()), (0, "Bash command\ncargo test\nProceed?\n1. Yes\n2. No\n"));
    assert_eq!(o.calls.iter().map(|c| c["params"]["lines"].clone()).collect::<Vec<_>>(), [json!(40)]);
    // Not blocked, or finished: "no prompt", and the screen is not read.
    for name in ["t1", "t2"] {
        let o = agents(&fx, &dir, None, &[], &["prompt", name]);
        assert_eq!((o.code, o.out.as_str(), o.calls.len()), (0, "no prompt\n", 0));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn git(dir: &Path, args: &[&str]) -> String {
    let run = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-C"]).arg(dir).args(args).output().unwrap();
    assert!(run.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&run.stderr));
    String::from_utf8_lossy(&run.stdout).trim().to_owned()
}

#[test]
fn review_pr_makes_a_detached_worktree_at_the_pr_head_and_starts_the_worker_there() {
    let dir = scratch("review");
    let (repo, fx) = (dir.join("repo"), fixture(&dir));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "one"]);
    let pr_head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "two"]);
    let gh = dir.join("gh");
    std::fs::write(&gh, format!("#!/bin/sh\necho \"$@\" > '{}'\necho '{{\"headRefOid\":\"{pr_head}\"}}'\n", dir.join("gh.args").display())).unwrap();
    Command::new("chmod").arg("+x").arg(&gh).status().unwrap();

    let args = ["spawn", "--name", "r-7", "--profile", "builder", "--no-parent", "--review-pr", "7", "--worktree", "repo", "review", "it"];
    let o = agents(&fx, &dir, None, &[("CONTR_GH", &gh)], &args);
    assert_eq!((o.code, o.err.as_str()), (0, ""));
    let wt = dir.join("repo-wt/review-7");
    assert_eq!(o.calls[0]["params"]["cwd"], wt.display().to_string());
    assert_eq!(o.calls[0]["params"]["labels"]["worktree"], wt.display().to_string());
    assert_eq!(std::fs::read_to_string(dir.join("gh.args")).unwrap(), "pr view 7 --json headRefOid\n");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), pr_head);
    assert_eq!(git(&wt, &["branch", "--show-current"]), "");

    // A second review of the same PR moves the existing worktree to the new head.
    let new_head = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(&gh, format!("#!/bin/sh\necho '{{\"headRefOid\":\"{new_head}\"}}'\n")).unwrap();
    let o = agents(&fx, &dir, None, &[("CONTR_GH", &gh)], &["spawn", "--name", "r-7b", "--profile", "builder", "--no-parent", "--review-pr", "7", "--worktree", "repo"]);
    assert_eq!(o.code, 0, "{}", o.err);
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), new_head);

    // gh failing, and a number that is not positive, are errors; a word is a usage error.
    std::fs::write(&gh, "#!/bin/sh\necho 'no pull requests found' >&2\nexit 1\n").unwrap();
    let o = agents(&fx, &dir, None, &[("CONTR_GH", &gh)], &["spawn", "--name", "r-8", "--profile", "builder", "--review-pr", "8", "--worktree", "repo"]);
    assert_eq!((o.code, o.err.as_str(), o.calls.len()), (1, "contr-rune agents spawn: agents: pull request 8: no pull requests found\n", 0));
    let o = agents(&fx, &dir, None, &[("CONTR_GH", &gh)], &["spawn", "--name", "r", "--profile", "builder", "--review-pr=-1", "--worktree", "repo"]);
    assert_eq!((o.code, o.err.as_str()), (1, "contr-rune agents spawn: agents: a pull request number is positive\n"));
    let o = agents(&fx, &dir, None, &[], &["spawn", "--review-pr", "x"]);
    assert_eq!(o.code, 2);
    std::fs::remove_dir_all(dir).unwrap();
}
