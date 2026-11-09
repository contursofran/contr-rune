//! A new orchestrator (A3) against the in-process fake core and a scratch `CONTR_AGENTS_CONFIG`:
//! the Projects `+` menu (`add-project/menu-item0|1`), the menu of an empty project row
//! (`project-<i>/menu-item0`), the logged `terminal.run` params and the error line.

mod common;

use common::*;
use serde_json::{Value, json};

const WITH: &str = r#"{"profiles":{"orchestrator":{"argv":["claude","--model","opus"]}}}"#;
const WITHOUT: &str = r#"{"profiles":{"builder":{"argv":["claude"]}}}"#;

/// `repo` (three terminals) and `empty` (nothing open), a folder `added` for the picker.
fn app(config: &'static str) -> App {
    App::new()
        .scene("three-tabs")
        .project("repo")
        .projects(&["empty"])
        .fixture_with(|_| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            fx["terminals"] = json!([]);
            fx
        })
        .file("CONTR_AGENTS_CONFIG", "agents.json", config)
        .setup(|d| std::fs::create_dir_all(format!("{}/added", d.root)).unwrap())
        .end(1.5)
}

fn right_click(t: f64, key: &str) -> Value {
    json!({"t": t, "type": "secondary-click", "key": key})
}

fn label(r: &Run, item: &str) -> Option<String> {
    r.child(item, "label").and_then(|e| e["text"].as_str().map(str::to_owned))
}

/// The `terminal.run` calls that start an orchestrator (the scene's own tabs run plain terminals).
fn orch_runs(r: &Run) -> Vec<&Value> {
    r.calls("terminal.run").into_iter().filter(|c| c["params"]["labels"]["role"] == "orchestrator").collect()
}

/// The one orchestrator run is the orchestrator of the folder named `folder`.
fn check_run(r: &Run, folder: &str) {
    let calls = orch_runs(r);
    assert_eq!(calls.len(), 1, "{:?}", r.log);
    let p = &calls[0]["params"];
    let name = p["name"].as_str().unwrap();
    assert!(name.starts_with(&format!("orch-{folder}-")) && name.len() == "orch-".len() + folder.len() + 5, "{name}");
    assert_eq!(p["labels"]["role"], "orchestrator");
    assert_eq!(p["labels"]["profile"], "orchestrator");
    assert_eq!(p["labels"]["project"], folder);
    assert_eq!(p["labels"]["contr"], "agent");
    assert!(p["cwd"].as_str().unwrap().ends_with(&format!("/{folder}")));
    assert_eq!(p["labels"]["worktree"], p["cwd"]);
    assert_eq!(p["no_parent"], true);
    assert_eq!(p["outlive_parent"], true);
    assert_eq!(p["argv"][0], "claude");
}

#[test]
fn the_plus_menu_has_new_orchestrator_and_open_folder() {
    let r = app(WITH).events(vec![tap(0.5, "add-project")]).run();
    assert!(r.has("add-project/menu-menu"));
    let rect = r.rect("add-project/menu-menu");
    let (a, b) = (r.rect("add-project/menu-item0"), r.rect("add-project/menu-item1"));
    assert!(a[1] >= rect[1] && b[1] > a[1] && b[1] + b[3] <= rect[1] + rect[3], "{rect:?} {a:?} {b:?}");
    assert_eq!(label(&r, "add-project/menu-item0").as_deref(), Some("New orchestrator…"));
    assert_eq!(label(&r, "add-project/menu-item1").as_deref(), Some("Open folder…"));
    assert!(orch_runs(&r).is_empty());
}

#[test]
fn new_orchestrator_picks_a_folder_and_runs_the_profile() {
    let r = app(WITH).events(vec![tap(0.5, "add-project"), tap(0.7, "add-project/menu-item0"), json!({"t": 0.9, "type": "file-dialog", "path": "{root}/added"})]).run();
    check_run(&r, "added");
    assert!(!r.has("error"));
    assert!(!r.has("add-project/menu-menu"), "the menu closes");
    assert_eq!(r.text_in("project-2", "label").as_deref(), Some("added"));
}

#[test]
fn a_cancelled_picker_runs_nothing() {
    let r = app(WITH).events(vec![tap(0.5, "add-project"), tap(0.7, "add-project/menu-item0"), json!({"t": 0.9, "type": "file-dialog-cancel"})]).run();
    assert!(orch_runs(&r).is_empty());
    assert!(!r.has("error"));
}

#[test]
fn an_empty_project_row_menu_starts_an_orchestrator_in_it() {
    let r = app(WITH).events(vec![right_click(0.5, "project-1")]).run();
    assert_eq!(label(&r, "project-1/menu-item0").as_deref(), Some("New orchestrator in empty"));
    let r = app(WITH).events(vec![right_click(0.5, "project-1"), tap(0.9, "project-1/menu-item0")]).run();
    check_run(&r, "empty");
    assert!(!r.has("error"));
}

#[test]
fn a_project_with_terminals_has_no_menu() {
    let r = app(WITH).events(vec![right_click(0.5, "project-0")]).run();
    assert!(!r.has("project-0/menu-menu"));
}

#[test]
fn no_orchestrator_profile_shows_the_error_and_runs_nothing() {
    let r = app(WITHOUT).events(vec![right_click(0.5, "project-1"), tap(0.9, "project-1/menu-item0")]).run();
    assert!(orch_runs(&r).is_empty());
    let e = r.find("error").and_then(|e| e["text"].as_str()).unwrap_or("").to_owned();
    assert!(e.starts_with("New orchestrator: no \"orchestrator\" profile in ") && e.ends_with("agents.json"), "{e}");
}
