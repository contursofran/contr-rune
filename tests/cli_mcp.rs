//! `contr-rune mcp` as a child process: JSON-RPC over stdio against the fake core
//! (`CONTR_HC_FAKE`). No window, no real hc.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const MENU: &str = "● Running\n╭────────────────╮\n│ Bash command   │\n│   cargo test   │\n│ Proceed?       │\n│ ❯ 1. Yes       │\n│   2. No        │\n╰────────────────╯\n";

fn terminal(address: &str, name: &str, parent: Option<&str>, state: &str, status: &str) -> Value {
    json!({"address": address, "name": name, "parent": parent, "argv": ["claude"],
        "labels": {"contr": "agent", "role": "builder"}, "state": state, "turn": 1,
        "foreground": {"pid": 1, "name": "claude", "argv": [], "cwd": "/tmp"},
        "status": {"state": status, "kind": if status == "blocked" { json!("permission") } else { Value::Null }, "msg": "", "title": "", "app": "claude-code", "progress": null}})
}

/// Sends `requests` (one per line) to `contr-rune mcp` and returns the replies and the fake core's call log.
fn session(requests: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let dir = std::env::temp_dir().join(format!("contr-rune-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("wt")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("agents.json"), "{}").unwrap();
    let terminals = [
        terminal("t1", "b-live", Some("t3"), "live", "working"),
        terminal("t2", "b-done", Some("t3"), "finished", "idle"),
        terminal("t3", "orch", None, "live", "idle"),
        terminal("t6", "b-block", Some("t3"), "live", "blocked"),
    ];
    let fixture = dir.join("fixture.json");
    std::fs::write(&fixture, json!({"terminals": terminals, "screens": {"t1": "line one\nline two\n", "t6": MENU}}).to_string()).unwrap();
    let log = dir.join("fake.log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .arg("mcp")
        .current_dir(&dir)
        .env("CONTR_HC_FAKE", &fixture)
        .env("CONTR_HC_FAKE_LOG", &log)
        .env("CONTR_AGENTS_CONFIG", dir.join("agents.json"))
        .env("CONTR_KIT", dir.join("kit"))
        .env("HC_TERMINAL_ID", "t3")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let run = child.wait_with_output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let replies = String::from_utf8(run.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let calls = std::fs::read_to_string(&log).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    std::fs::remove_dir_all(dir).unwrap();
    (replies, calls)
}

fn tool(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn lists_the_tools_and_runs_one_call_per_tool_group() {
    let (r, calls) = session(&[
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        tool(2, "message", json!({"name": "t1", "text": "fix it"})),
        tool(3, "message", json!({"name": "t1", "message": "fix it"})),
        tool(4, "read", json!({"name": "t1", "lines": 5})),
        tool(5, "prompt", json!({"name": "t6"})),
        tool(6, "list", json!({})),
        tool(7, "clear_ended", json!({})),
        tool(8, "stop", json!({"name": "t1", "keep": true})),
        tool(9, "spawn", json!({"name": "b-9", "profile": "builder", "task": "do it", "worktree": "wt"})),
    ]);
    assert_eq!(r.len(), 10, "the notification gets no reply");
    assert_eq!(r[0]["result"]["serverInfo"]["name"], "contr");
    let names: Vec<&str> = r[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["spawn", "message", "interrupt", "stop", "read", "prompt", "list", "clear_ended"]);

    assert_eq!(text(&r[2]), "sent");
    assert_eq!(r[3]["result"]["isError"], true);
    assert!(text(&r[3]).starts_with("message: bad arguments: unknown field `message`"), "{}", text(&r[3]));
    assert!(text(&r[4]).contains("line two"));
    assert!(text(&r[5]).contains("Proceed?") && text(&r[5]).contains("1. Yes"), "{}", text(&r[5]));
    let listed: Value = serde_json::from_str(text(&r[6])).unwrap();
    assert!(listed.as_array().unwrap().iter().any(|w| w["name"] == "b-live"), "{listed}");
    assert_eq!(text(&r[7]), "[\n  \"b-done\"\n]");
    assert_eq!(text(&r[8]), "stopped");
    assert_eq!(r[9]["result"]["isError"], Value::Null, "{}", text(&r[9]));
    assert_eq!(serde_json::from_str::<Value>(text(&r[9])).unwrap()["name"], "b-9");

    let methods: Vec<&str> = calls.iter().map(|c| c["method"].as_str().unwrap()).collect();
    assert!(methods.contains(&"terminal.send") && methods.contains(&"terminal.run") && methods.contains(&"terminal.kill"), "{methods:?}");
    assert_eq!(calls.iter().filter(|c| c["method"] == "terminal.send").count(), 1, "the refused call never reached the core");
}
