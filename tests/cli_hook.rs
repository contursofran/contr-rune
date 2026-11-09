//! `contr-rune hook stop` against the fake core (`CONTR_HC_FAKE`): a hook JSON on stdin, the
//! logged `terminal.send` and `terminal.label`. No window, no real hc, no real transcripts.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-hook-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

const PROMPT: &str = r#"{"type":"user","message":{"content":"build it"}}"#;
const SAY: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"text","text":"all done,\nPR is up"}]}}"#;
const SEND: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"hc send t3 done"}}]}}"#;

struct Out {
    code: i32,
    err: String,
    calls: Vec<Value>,
}

/// Runs `contr-rune hook stop` with `transcript` lines, in the worker `t1` of coordinator `t3`
/// (or as the session `t3` when `worker` is false).
fn stop(name: &str, transcript: &[&str], worker: bool, extra: Value) -> Out {
    let dir = scratch(name);
    let fixture = dir.join("fixture.json");
    let terminal = |a: &str, n: &str| json!({"address": a, "name": n, "state": "live", "turn": 1, "labels": {}});
    std::fs::write(&fixture, json!({"terminals": [terminal("t1", "worker"), terminal("t3", "orch")]}).to_string()).unwrap();
    let path = dir.join("t.jsonl");
    std::fs::write(&path, transcript.join("\n")).unwrap();
    let log = dir.join("fake.log");
    let mut input = json!({"session_id": "s", "transcript_path": path, "cwd": dir});
    input.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.args(["hook", "stop"]).current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("CONTR_HC_FAKE", &fixture).env("CONTR_HC_FAKE_LOG", &log).env_remove("CLAUDE_EFFORT");
    cmd.env("HC_TERMINAL_ID", if worker { "t1" } else { "t3" });
    if worker {
        cmd.env("CONTR_COORDINATOR", "t3");
    } else {
        cmd.env_remove("CONTR_COORDINATOR");
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let run = child.wait_with_output().unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    Out { code: run.status.code().unwrap(), err: String::from_utf8_lossy(&run.stderr).into_owned(), calls }
}

fn label_call(model_and_effort: Value) -> Value {
    json!({"method": "terminal.label", "params": {"terminal": "t1", "labels": model_and_effort}})
}

#[test]
fn a_worker_that_sent_no_report_gets_one_sent_and_is_labelled() {
    let o = stop("missed", &[PROMPT, SAY], true, json!({"effort": {"level": "high"}}));
    assert_eq!((o.code, o.err.as_str()), (0, ""));
    assert_eq!(
        o.calls,
        [
            label_call(json!({"contr.effort": "high", "contr.model": "sonnet"})),
            json!({"method": "terminal.send", "params": {"terminal": "t3", "text": "ended its turn without a report. Last words: all done, PR is up", "caller": "t1"}}),
        ]
    );
}

#[test]
fn a_worker_that_reported_is_only_labelled() {
    let o = stop("reported", &[PROMPT, SEND, SAY], true, json!({}));
    assert_eq!((o.code, o.err.as_str()), (0, ""));
    assert_eq!(o.calls, [label_call(json!({"contr.model": "sonnet"}))]);
}

#[test]
fn a_session_that_is_not_a_worker_is_labelled_and_sends_nothing() {
    let o = stop("session", &[PROMPT, SAY], false, json!({"last_assistant_message": "x"}));
    assert_eq!(o.code, 0);
    assert_eq!(o.calls, [json!({"method": "terminal.label", "params": {"terminal": "t3", "labels": {"contr.model": "sonnet"}}})]);
}

#[test]
fn a_missing_transcript_exits_1_and_never_2() {
    let o = stop("missing", &[PROMPT], true, json!({"transcript_path": "/nonexistent/t.jsonl"}));
    assert_eq!(o.code, 1);
    assert!(o.err.contains("transcript"), "{}", o.err);
}
