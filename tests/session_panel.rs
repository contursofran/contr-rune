//! The session panel over the file tree (C4): the selected agent's todos and artifacts from its
//! K2 session file (`<CONTR_STATE_DIR>/session/<id>.json`, a fixture here). Keys: `session`,
//! `session/todos` (+ `/head`, `/count`), `session/todo/<i>`, `session/artifacts`,
//! `session/artifact/<i>` (with an `age` child). An artifact press appends its URL to `CONTR_OPEN_LOG`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use common::*;
use serde_json::{Value, json};

fn run(session: Option<Value>, events: Vec<Value>) -> Run {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("contr-c4-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&root);
    if let Some(s) = session {
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::write(root.join("session/t3.json"), s.to_string()).unwrap();
    }
    let r = App::new()
        .project("contr")
        .env("CONTR_STATE_DIR", root.to_string_lossy())
        .file("CONTR_OPEN_LOG", "opened.log", "")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx
        })
        .events(events)
        .end(1.5)
        .run();
    let _ = std::fs::remove_dir_all(&root);
    r
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn file(todos: Value, artifacts: Value) -> Value {
    json!({"schema": 1, "updated_at": now(), "todos": todos, "artifacts": artifacts})
}

#[test]
fn todos_and_artifacts_sit_between_the_head_and_the_tree() {
    let n = now();
    let s = file(
        json!([
            {"text": "Read the plan", "active": "Reading the plan", "status": "completed", "you": false},
            {"text": "Write the code", "active": "Writing the code", "status": "in_progress", "you": false},
            {"text": "Ship it", "active": "", "status": "pending", "you": true}
        ]),
        json!([
            {"slug": "a", "title": "Plan board", "url": "https://example.com/a", "updated_at": n},
            {"slug": "b", "title": "Report", "url": "https://example.com/b", "updated_at": n - 38 * 60}
        ]),
    );
    let t = run(Some(s), vec![tap(0.5, "agent/t3")]);
    assert_eq!(t.find("files/title").unwrap()["text"], "Files");
    assert_eq!(t.text_in("session/todos/head", "title").as_deref(), Some("Todos"));
    assert_eq!(t.text_in("session/todos/head", "count").as_deref(), Some("1 of 3"));
    // The in-progress todo shows its active form.
    assert_eq!(t.text_in("session/todo/1", "text").as_deref(), Some("Writing the code"));
    assert_eq!(t.text_in("session/todo/2", "you").as_deref(), Some("you"));
    assert_eq!(t.text_in("session/artifacts/head", "title").as_deref(), Some("Artifacts"));
    assert_eq!(t.text_in("session/artifacts/head", "count").as_deref(), Some("2"));
    assert_eq!(t.text_in("session/artifact/0", "age").as_deref(), Some("now"));
    assert_eq!(t.text_in("session/artifact/1", "age").as_deref(), Some("38m"));
    let (head, session, tree) = (t.rect("files/head"), t.rect("session"), t.rect("tree"));
    assert!(session[1] >= head[1] + head[3] && tree[1] >= session[1] + session[3] - 1.0, "{head:?} {session:?} {tree:?}");
    assert!(t.rect("session/artifacts")[1] > t.rect("session/todos")[1]);
}

#[test]
fn an_empty_section_is_hidden() {
    let only_todos = run(Some(file(json!([{"text": "x", "active": "", "status": "pending", "you": false}]), json!([]))), vec![tap(0.5, "agent/t3")]);
    assert!(only_todos.has("session/todos") && !only_todos.has("session/artifacts"));
    let only_artifacts = run(Some(file(json!([]), json!([{"slug": "a", "title": "A", "url": "https://example.com/a", "updated_at": 1}]))), vec![tap(0.5, "agent/t3")]);
    assert!(only_artifacts.has("session/artifacts") && !only_artifacts.has("session/todos"));
    assert!(only_artifacts.text_in("session/artifact/0", "age").is_some_and(|a| a.ends_with('d')));
    // Both empty, no file, or no agent selected: no panel, and the tree stays.
    assert!(!run(Some(file(json!([]), json!([]))), vec![tap(0.5, "agent/t3")]).has("session"));
    assert!(!run(None, vec![tap(0.5, "agent/t3")]).has("session"));
    assert!(!run(Some(file(json!([{"text": "x", "active": "", "status": "pending", "you": false}]), json!([]))), vec![]).has("session"));
}

#[test]
fn an_artifact_press_opens_its_url() {
    let s = file(json!([]), json!([{"slug": "a", "title": "Plan board", "url": "https://example.com/a", "updated_at": now()}, {"slug": "b", "title": "Report", "url": "https://example.com/b", "updated_at": now()}]));
    let t = run(Some(s), vec![tap(0.5, "agent/t3"), tap(1.0, "session/artifact/1")]);
    assert_eq!(t.file("opened.log"), "https://example.com/b\n");
}
