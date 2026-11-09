//! `contr-rune hook stop` (coordinator: ask for COORDINATOR-STATE.md once) and `hook
//! session-start` (after a compaction) against the fake core, a scratch state folder, a scratch
//! kit and a scratch owner file. No window, no real hc, no real transcripts.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-compact-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

const PROMPT: &str = r#"{"type":"user","message":{"content":"build it"}}"#;
const ORCHESTRATE: &str = r#"{"type":"user","message":{"role":"user","content":"<command-name>/contr:orchestrate</command-name> the goal"}}"#;

fn usage(tokens: u64) -> String {
    format!(r#"{{"type":"assistant","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":10,"cache_read_input_tokens":{},"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"ok"}}]}}}}"#, tokens - 10)
}

/// A scratch world: the fake core with the coordinator `t3` (role `role`) and its worker `t1`, a
/// state folder, a kit with a playbook, an owner file.
struct World {
    dir: PathBuf,
    transcript: PathBuf,
}

impl World {
    fn new(name: &str, role: &str, transcript: &[String]) -> World {
        let dir = scratch(name);
        let terminal = |a: &str, n: &str, parent: Value, role: &str| {
            json!({"address": a, "name": n, "state": "live", "turn": 1, "parent": parent, "labels": {"contr": "agent", "role": role, "issue": "#7", "worktree": "/wt/one"}})
        };
        let fixture = json!({"terminals": [terminal("t1", "worker", json!("t3"), "builder"), terminal("t3", "orch", Value::Null, role)]});
        std::fs::write(dir.join("fixture.json"), fixture.to_string()).unwrap();
        let transcript_path = dir.join("t.jsonl");
        std::fs::write(&transcript_path, transcript.join("\n")).unwrap();
        std::fs::create_dir_all(dir.join("kit/commands")).unwrap();
        std::fs::write(dir.join("kit/commands/orchestrate.md"), "---\ndescription: x\n---\nPLAYBOOK for $ARGUMENTS\n").unwrap();
        let owner = json!({"questions": [
            {"id": "D1", "coordinator": "t3", "title": "Ship it?", "question": "long", "answer": ""},
            {"id": "D2", "coordinator": "t9", "title": "Not mine", "answer": ""},
            {"id": "D3", "coordinator": "t3", "title": "Answered", "answer": "yes"}],
            "todos": [{"id": "T1", "coordinator": "t3", "kind": "do", "text": "run the app"}]});
        std::fs::write(dir.join("owner.json"), owner.to_string()).unwrap();
        World { dir, transcript: transcript_path }
    }

    fn marker(&self, session: &str) -> PathBuf {
        self.dir.join("state/hooks").join(format!("asked-{session}"))
    }

    /// Runs `contr-rune hook <hook>` as the session `t3`; returns (exit code, stdout, stderr).
    fn run(&self, hook: &str, input: Value, envs: &[(&str, &str)]) -> (i32, String, String) {
        let mut input_json = json!({"session_id": "s1", "transcript_path": self.transcript, "cwd": self.dir});
        input_json.as_object_mut().unwrap().extend(input.as_object().unwrap().clone());
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
        cmd.args(["hook", hook]).current_dir(&self.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.env("CONTR_HC_FAKE", self.dir.join("fixture.json")).env("CONTR_HC_FAKE_LOG", self.dir.join("fake.log"));
        cmd.env("CONTR_STATE_DIR", self.dir.join("state")).env("CONTR_OWNER_FILE", self.dir.join("owner.json"));
        cmd.env("CONTR_AGENTS_CONFIG", self.dir.join("none.json")).env("CONTR_KIT", self.dir.join("kit"));
        cmd.env("HC_TERMINAL_ID", "t3").env_remove("CONTR_COORDINATOR").env_remove("CONTR_SAVE_AT").env_remove("CLAUDE_PLUGIN_ROOT").env_remove("CLAUDE_EFFORT");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(input_json.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn decision(out: &str) -> Value {
    serde_json::from_str(out).unwrap_or_else(|e| panic!("not one JSON value ({e}): {out:?}"))
}

#[test]
fn a_coordinator_past_the_save_size_is_asked_once_per_session() {
    let w = World::new("once", "orchestrator", &[PROMPT.into(), usage(230_000)]);
    let (code, out, err) = w.run("stop", json!({}), &[]);
    assert_eq!((code, err.as_str()), (0, ""));
    let d = decision(&out);
    assert_eq!(d["decision"], "block");
    let reason = d["reason"].as_str().unwrap();
    assert!(reason.contains("230k tokens") && reason.contains("Update COORDINATOR-STATE.md now"), "{reason}");
    assert!(w.marker("s1").exists());
    // The same session is not asked again; another session is.
    assert_eq!(w.run("stop", json!({}), &[]).1, "");
    assert!(w.run("stop", json!({"session_id": "s2"}), &[]).1.contains("block"));
}

#[test]
fn the_save_size_comes_from_contr_save_at_and_a_kept_turn_is_not_asked_again() {
    let w = World::new("size", "orchestrator", &[PROMPT.into(), usage(100_000)]);
    assert_eq!(w.run("stop", json!({}), &[]).1, "", "100k is under the default 220k");
    assert_eq!(w.run("stop", json!({"stop_hook_active": true}), &[("CONTR_SAVE_AT", "50000")]).1, "");
    assert!(!w.marker("s1").exists());
    let (_, out, _) = w.run("stop", json!({}), &[("CONTR_SAVE_AT", "50000")]);
    assert!(decision(&out)["reason"].as_str().unwrap().contains("100k tokens"));
}

#[test]
fn a_session_that_ran_orchestrate_is_a_coordinator_and_any_other_session_is_left_alone() {
    let w = World::new("who", "terminal", &[ORCHESTRATE.into(), usage(230_000)]);
    assert!(w.run("stop", json!({}), &[]).1.contains("block"), "ran /contr:orchestrate");
    let plain = World::new("plain", "terminal", &[PROMPT.into(), usage(230_000)]);
    let (code, out, err) = plain.run("stop", json!({}), &[]);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    assert!(!plain.marker("s1").exists());
}

#[test]
fn a_worker_is_never_asked_for_a_state_file() {
    let w = World::new("worker", "orchestrator", &[PROMPT.into(), usage(230_000), r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"hc send t3 done"}}]}}"#.into()]);
    let (code, out, _) = w.run("stop", json!({}), &[("CONTR_COORDINATOR", "t3")]);
    assert_eq!((code, out.as_str()), (0, ""));
    assert!(!w.marker("s1").exists());
}

#[test]
fn session_start_after_a_compaction_prints_the_restore_text_and_clears_the_marker() {
    let w = World::new("restore", "orchestrator", &[PROMPT.into(), usage(230_000)]);
    assert!(w.run("stop", json!({}), &[]).1.contains("block"));
    assert!(w.marker("s1").exists());
    let (code, out, err) = w.run("session-start", json!({"source": "compact"}), &[]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.starts_with("contr: the conversation was compacted."), "{out}");
    assert!(out.contains(&format!("First read {}/COORDINATOR-STATE.md", w.dir.display())), "{out}");
    assert!(out.contains("Your live workers:\n- worker (t1) builder, "), "{out}");
    assert!(out.contains(", issue #7, /wt/one\n"), "{out}");
    assert!(out.contains("Your open owner items:\n- D1 Ship it?\n- T1 [do] run the app\n"), "{out}");
    assert!(!out.contains("Not mine") && !out.contains("Answered"));
    assert!(out.ends_with("The playbook:\n\nPLAYBOOK for (see the state file)"), "{out}");
    assert!(!w.marker("s1").exists(), "the next compaction is asked for again");
}

#[test]
fn session_start_is_capped_and_only_runs_for_compact_in_a_coordinator() {
    let w = World::new("cap", "orchestrator", &[PROMPT.into()]);
    std::fs::write(w.dir.join("kit/commands/orchestrate.md"), format!("---\nx: y\n---\n{}\n", "play ".repeat(5000))).unwrap();
    let (code, out, _) = w.run("session-start", json!({"source": "compact"}), &[("CLAUDE_PLUGIN_ROOT", w.dir.join("kit").to_str().unwrap())]);
    assert_eq!(code, 0);
    assert_eq!(out.chars().count(), 9500);
    assert!(out.ends_with('…') && out.contains("Your open owner items:"));
    // Another source prints nothing and leaves the marker alone.
    std::fs::create_dir_all(w.dir.join("state/hooks")).unwrap();
    std::fs::write(w.marker("s1"), "").unwrap();
    for source in ["startup", "resume", "clear"] {
        assert_eq!(w.run("session-start", json!({"source": source}), &[]).1, "", "{source}");
    }
    assert!(w.marker("s1").exists());
    // A session that is not a coordinator prints nothing.
    let plain = World::new("plain-start", "terminal", &[PROMPT.into()]);
    assert_eq!(plain.run("session-start", json!({"source": "compact"}), &[]), (0, String::new(), String::new()));
    // Outside hc it is a no-op.
    let (code, out, _) = w.run("session-start", json!({"source": "compact"}), &[("HC_TERMINAL_ID", "")]);
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn a_bad_session_start_input_exits_1_and_never_2() {
    let w = World::new("bad", "orchestrator", &[PROMPT.into()]);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.args(["hook", "session-start"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("CONTR_HC_FAKE", w.dir.join("fixture.json")).env("HC_TERMINAL_ID", "t3");
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"nope").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad hook input"));
}
