//! `contr-rune owner ask | answer | done` against a scratch owner file (`CONTR_OWNER_FILE`) and
//! the fake core (`CONTR_HC_FAKE`): no window, never the owner's real file or hc.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-owner-{}-{name}", std::process::id()));
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

/// A fake core with a live coordinator `t3` and a finished terminal `t2`.
fn fixture(dir: &Path) -> PathBuf {
    let terminal = |address: &str, name: &str, state: &str| {
        json!({"address": address, "name": name, "parent": null, "argv": ["claude"], "labels": {}, "state": state, "turn": 1,
            "foreground": {"pid": 1, "name": "claude", "argv": [], "cwd": "/tmp"},
            "status": {"state": "idle", "kind": null, "msg": "", "title": "", "app": "claude-code", "progress": null}})
    };
    let path = dir.join("fixture.json");
    std::fs::write(&path, json!({"terminals": [terminal("t3", "orch", "live"), terminal("t2", "gone", "finished")]}).to_string()).unwrap();
    path
}

/// Runs `contr-rune owner <args>` in `dir`, as the terminal `terminal_id` when given.
fn owner(dir: &Path, terminal_id: Option<&str>, project: Option<&str>, args: &[&str]) -> Out {
    let log = dir.join("fake.log");
    let _ = std::fs::remove_file(&log);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.arg("owner").args(args).current_dir(dir);
    cmd.env("CONTR_HC_FAKE", dir.join("fixture.json")).env("CONTR_HC_FAKE_LOG", &log).env("CONTR_OWNER_FILE", dir.join("owner.json"));
    match terminal_id {
        Some(id) => cmd.env("HC_TERMINAL_ID", id),
        None => cmd.env_remove("HC_TERMINAL_ID"),
    };
    match project {
        Some(p) => cmd.env("CONTR_PROJECT", p),
        None => cmd.env_remove("CONTR_PROJECT"),
    };
    let run = cmd.output().unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    Out { code: run.status.code().unwrap(), out: String::from_utf8_lossy(&run.stdout).into_owned(), err: String::from_utf8_lossy(&run.stderr).into_owned(), calls }
}

fn file(dir: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join("owner.json")).unwrap()).unwrap()
}

#[test]
fn ask_prints_the_id_and_saves_the_question_for_the_project_and_coordinator() {
    let dir = scratch("ask");
    fixture(&dir);
    let o = owner(&dir, Some("t3"), Some("contr"), &["ask", "--title", "Ship it", "--option", "yes", "--option", "no", "--impact", "fast", "--recommended", "0", "--ref", "#12", "--source", "log line", "--from", "b-1", "Ship", "now?"]);
    assert_eq!((o.code, o.out.as_str(), o.err.as_str()), (0, "D1\n", ""));
    let q = &file(&dir)["questions"][0];
    for (key, want) in [
        ("id", json!("D1")),
        ("project", json!("contr")),
        ("coordinator", json!("t3")),
        ("from", json!("b-1")),
        ("title", json!("Ship it")),
        ("question", json!("Ship now?")),
        ("options", json!(["yes", "no"])),
        ("recommended", json!(0)),
        ("impacts", json!(["fast"])),
        ("refs", json!(["#12"])),
        ("source", json!("log line")),
    ] {
        assert_eq!(q[key], want, "{key}");
    }
    assert!(o.calls.is_empty(), "ask does not touch the core");

    // Without CONTR_PROJECT the project is the folder; without a terminal there is no coordinator.
    let o = owner(&dir, None, None, &["ask", "Second?"]);
    assert_eq!((o.code, o.out.as_str()), (0, "D2\n"));
    let q = &file(&dir)["questions"][1];
    assert_eq!(q["project"], dir.file_name().unwrap().to_str().unwrap());
    assert_eq!(q.get("coordinator"), None);
}

#[test]
fn answer_and_done_save_first_then_tell_the_coordinator() {
    let dir = scratch("tell");
    fixture(&dir);
    std::fs::write(
        dir.join("owner.json"),
        json!({"questions": [
            {"id": "D1", "project": "p", "coordinator": "t3", "question": "Ship?", "asked": "2026-11-01T10:00:00Z"},
            {"id": "D2", "project": "p", "question": "Colour?", "asked": "2026-11-01T10:00:00Z"},
            {"id": "D3", "project": "p", "coordinator": "t2", "question": "Gone?", "asked": "2026-11-01T10:00:00Z"}],
            "todos": [{"id": "T1", "project": "p", "coordinator": "t3", "kind": "do", "text": "Rotate the key", "pinned": "2026-11-01T10:00:00Z"}],
            "next_question": 4, "next_todo": 2})
        .to_string(),
    )
    .unwrap();

    let o = owner(&dir, None, None, &["answer", "D1", "ship", "it"]);
    assert_eq!((o.code, o.out.as_str(), o.err.as_str()), (0, "", ""));
    assert_eq!(o.calls, [json!({"method": "terminal.send", "params": {"terminal": "t3", "text": "Owner answered D1: ship it\n"}})]);
    let d1 = &file(&dir)["questions"][0];
    assert_eq!((d1["answer"].as_str(), d1["answered_by"].as_str()), (Some("ship it"), Some("owner")));

    let o = owner(&dir, None, None, &["done", "T1"]);
    assert_eq!((o.code, o.err.as_str()), (0, ""));
    assert_eq!(o.calls, [json!({"method": "terminal.send", "params": {"terminal": "t3", "text": "Owner did T1: Rotate the key\n"}})]);
    assert!(file(&dir)["todos"][0]["done"].as_str().is_some_and(|t| t.ends_with('Z')));

    // No coordinator: saved, nothing to tell, exit 0.
    let o = owner(&dir, None, None, &["answer", "D2", "blue"]);
    assert_eq!((o.code, o.calls.len()), (0, 0));
    assert_eq!(file(&dir)["questions"][1]["answer"], "blue");

    // A coordinator the core refuses: the answer stays saved, exit 1.
    let o = owner(&dir, None, None, &["answer", "D3", "no"]);
    assert_eq!(o.code, 1);
    assert!(o.err.starts_with("contr-rune owner answer: saved; coordinator not reached: "), "{}", o.err);
    assert_eq!(file(&dir)["questions"][2]["answer"], "no");
}

#[test]
fn mistakes_have_the_exit_codes_of_go() {
    let dir = scratch("errors");
    fixture(&dir);
    let code = |args: &[&str]| {
        let o = owner(&dir, None, None, args);
        (o.code, o.err)
    };
    // Usage errors: exit 2 and the usage text.
    for args in [&["ask"][..], &["ask", "--title", "T"], &["ask", "--recommended", "0", "q"], &["answer", "D1"], &["answer", "D1", " "], &["done"], &["done", "T1", "T2"], &["bogus"], &[]] {
        let (c, err) = code(args);
        assert_eq!(c, 2, "{args:?}");
        assert!(err.contains("Usage:"), "{args:?}: {err}");
    }
    assert!(code(&["answer", "D1"]).1.starts_with("contr-rune owner answer: wrong number of arguments\n"));
    // An id that is not open: exit 1, and the file is not created.
    let (c, err) = code(&["answer", "D9", "x"]);
    assert_eq!((c, err.as_str()), (1, "contr-rune owner answer: no open question D9\n"));
    let (c, err) = code(&["done", "T9"]);
    assert_eq!((c, err.as_str()), (1, "contr-rune owner done: no open to-do T9\n"));
    // help prints the usage on stdout.
    let o = owner(&dir, None, None, &["help"]);
    assert_eq!(o.code, 0);
    assert!(o.out.starts_with("Usage:\n  contr-rune owner ask"));
}
