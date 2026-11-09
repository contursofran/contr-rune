//! `contr-rune hook post-tool` and `hook prompt`: hook JSON on stdin, the session file in a
//! scratch state dir (`CONTR_STATE_DIR`). No window, no hc, nothing of the owner's.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-cli-hook-session-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Runs `contr-rune hook <kind>` with `input` on stdin. `env` sets (or, for `None`, removes) variables.
fn hook(state: &Path, kind: &str, input: &str, env: &[(&str, Option<&str>)]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.args(["hook", kind]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("CONTR_STATE_DIR", state).env("HC_TERMINAL_ID", "t/1").env_remove("CONTR_COORDINATOR");
    for (k, v) in env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn post(state: &Path, tool: &str, input: Value, response: Value) -> (i32, String) {
    let (code, _, err) = hook(state, "post-tool", &json!({"tool_name": tool, "tool_input": input, "tool_response": response}).to_string(), &[]);
    (code, err)
}

/// The id `t/1` is sanitised to `t_1`.
fn file(state: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(state.join("session/t_1.json")).unwrap()).unwrap()
}

#[test]
fn post_tool_keeps_todos_and_artifacts_in_the_go_shaped_file() {
    let state = scratch("file");
    let resp = json!({"task": {"id": "9"}});
    assert_eq!(post(&state, "TaskCreate", json!({"subject": "[you] ship", "activeForm": "Shipping"}), resp).0, 0);
    assert_eq!(post(&state, "TaskUpdate", json!({"taskId": "9", "status": "in_progress"}), Value::Null).0, 0);
    let art = "mcp__claude-artifacts__publish_version";
    assert_eq!(post(&state, art, json!({"slug": "plan", "title": "Plan"}), json!({"url": "https://claude.ai/a/plan"})).0, 0);
    assert_eq!(post(&state, art, json!({"slug": "other"}), Value::Null).0, 0);
    assert_eq!(post(&state, art, json!({"slug": "plan"}), Value::Null).0, 0);

    let v = file(&state);
    assert_eq!(v["schema"], 1);
    assert!(v["updated_at"].as_u64().unwrap() > 0);
    assert_eq!(v["todos"], json!([{"id": "9", "text": "ship", "active": "Shipping", "status": "in_progress", "you": true}]));
    let slugs: Vec<_> = v["artifacts"].as_array().unwrap().iter().map(|a| a["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["plan", "other"]);
    assert_eq!((v["artifacts"][0]["title"].as_str(), v["artifacts"][0]["url"].as_str()), (Some("Plan"), Some("https://claude.ai/a/plan")));

    // TodoWrite replaces the whole list; a tool that records nothing leaves the file alone.
    let todos = json!({"todos": [{"content": "a", "status": "pending", "activeForm": "A"}]});
    assert_eq!(post(&state, "TodoWrite", todos, Value::Null).0, 0);
    assert_eq!(file(&state)["todos"], json!([{"text": "a", "active": "A", "status": "pending", "you": false}]));
    let before = std::fs::read(state.join("session/t_1.json")).unwrap();
    assert_eq!(post(&state, "Bash", json!({"command": "ls"}), Value::Null).0, 0);
    assert_eq!(std::fs::read(state.join("session/t_1.json")).unwrap(), before);
    let leftovers: Vec<_> = std::fs::read_dir(state.join("session")).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temp file left");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn post_tool_caps_artifacts_at_20() {
    let state = scratch("cap");
    for i in 0..22 {
        post(&state, "mcp__claude-artifacts__read_artifact", json!({"slug": format!("s{i}")}), Value::Null);
    }
    let v = file(&state);
    assert_eq!(v["artifacts"].as_array().unwrap().len(), 20);
    assert_eq!(v["artifacts"][0]["slug"], "s21");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn post_tool_exits_0_on_bad_input_and_replaces_a_bad_file() {
    let state = scratch("bad");
    let (code, _, err) = hook(&state, "post-tool", "nope", &[]);
    assert_eq!(code, 0);
    assert!(err.contains("bad hook input"), "{err}");
    let (code, err) = post(&state, "TodoWrite", json!("nope"), Value::Null);
    assert_eq!(code, 0);
    assert!(err.contains("TodoWrite input"), "{err}");
    assert!(!state.join("session/t_1.json").exists());

    std::fs::create_dir_all(state.join("session")).unwrap();
    std::fs::write(state.join("session/t_1.json"), "not json").unwrap();
    let todos = json!({"todos": [{"content": "a", "status": "pending", "activeForm": "A"}]});
    assert_eq!(post(&state, "TodoWrite", todos, Value::Null).0, 0);
    assert_eq!(file(&state)["todos"][0]["text"], "a");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn hooks_do_nothing_outside_hc() {
    let state = scratch("outside");
    let input = json!({"tool_name": "TodoWrite", "tool_input": {"todos": []}, "prompt": "hi"}).to_string();
    let none = [("HC_TERMINAL_ID", None)];
    assert_eq!(hook(&state, "post-tool", &input, &none), (0, String::new(), String::new()));
    assert_eq!(hook(&state, "prompt", &input, &none), (0, String::new(), String::new()));
    assert!(!state.join("session").exists());
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn prompt_asks_for_you_todos_but_not_for_hc_prompts_or_workers() {
    let state = scratch("prompt");
    let (code, out, _) = hook(&state, "prompt", &json!({"prompt": "fix the login"}).to_string(), &[]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    assert!(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("[you] "));

    let hc = json!({"prompt": "<hc from=\"t3\">do this</hc>"}).to_string();
    assert_eq!(hook(&state, "prompt", &hc, &[]), (0, String::new(), String::new()));
    let worker = [("CONTR_COORDINATOR", Some("t3"))];
    assert_eq!(hook(&state, "prompt", &json!({"prompt": "fix"}).to_string(), &worker), (0, String::new(), String::new()));
    let _ = std::fs::remove_dir_all(&state);
}
