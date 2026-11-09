//! Tabs are hc terminals (H3), against the in-process fake core (`CONTR_HC_FAKE`, with the log in
//! `CONTR_HC_FAKE_LOG`). The new-tab chord and the first tab run a terminal with the Contract's
//! labels, the grid shows the fake's first paint, typed keys go out as DATA and the echo shows in
//! the grid, a split pane's resize goes out as RESIZE, closing a tab sends kill then rm, a CLOSE from
//! the core closes the tab, and a core that cannot run a terminal leaves a local PTY. Elements:
//! tabs `t-<id>`, grid rows `l<N>`. No test dials a real hc.

mod common;

use std::sync::OnceLock;

use common::*;
use serde_json::{Value, json};

fn app(events: Vec<Value>) -> Run {
    App::new().fixture("tabs.json").events(events).end(2.0).run()
}

fn enter(t: f64) -> Value {
    key(t, "Enter")
}

/// The launch with no input: shared by the tests that only read the first tab.
fn first_tab() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| app(vec![]))
}

#[test]
fn the_first_tab_runs_a_terminal_with_the_contract_params_and_shows_its_first_paint() {
    let r = first_tab();
    let runs = r.calls("terminal.run");
    assert_eq!(runs.len(), 1, "{:?}", r.log);
    let p = &runs[0]["params"];
    assert_eq!(p["argv"], json!([]), "empty argv: the core starts the login shell");
    assert_eq!(p["cwd"], r.project.as_str());
    assert_eq!(p["labels"], json!({"contr": "agent", "role": "terminal", "project": "proj", "worktree": r.project}));
    assert_eq!((&p["no_parent"], &p["outlive_parent"], &p["wait"]), (&json!(true), &json!(true), &json!(false)));
    assert_eq!(p["env"], json!({"TERM": "xterm-256color", "COLORTERM": "truecolor"}));
    assert!(p["size"]["cols"].as_u64().unwrap() > 0 && p["size"]["rows"].as_u64().unwrap() > 0);
    let name = p["name"].as_str().unwrap();
    assert!(name.starts_with("term-") && name[5..].chars().all(|c| c.is_ascii_digit()), "name {name}");
    assert_eq!(r.tabs(), vec!["t-1"]);
    assert!(r.grid().starts_with('$'), "first paint of the fake's new terminal: {:?}", r.grid());
}

#[test]
fn the_chord_and_the_plus_button_run_more_terminals() {
    let r = app(vec![chord(1.0, "t", false), tap(1.5, "new-tab")]);
    assert_eq!(r.calls("terminal.run").len(), 3);
    assert_eq!(r.tabs(), vec!["t-1", "t-2", "t-3"]);
    let names: Vec<&str> = r.calls("terminal.run").iter().map(|c| c["params"]["name"].as_str().unwrap()).collect();
    assert!(names[0] != names[1] && names[1] != names[2], "{names:?}");
}

#[test]
fn typed_keys_go_out_as_data_and_the_echo_shows_in_the_grid() {
    let r = app(vec![typed(1.0, "ls"), enter(1.5)]);
    assert_eq!(r.data("t1"), "ls\r");
    assert!(r.grid().contains("$ ls"), "echo: {:?}", r.grid());
}

#[test]
fn a_split_pane_resize_goes_out_as_resize() {
    let r = app(vec![tap(1.0, "split")]);
    assert_eq!(r.calls("terminal.run").len(), 2, "the split opens a second terminal");
    let left = r.resizes("t1");
    assert!(left.len() >= 2, "layout, then the split: {left:?}");
    let (before, after) = (left[left.len() - 2].0, left[left.len() - 1].0);
    assert!(after * 10 < before * 6, "the left pane is about half as wide: {before} then {after}");
    // The new pane's terminal hears its own size too (it was created with the full-panel size).
    let right = r.resizes("t2");
    assert!(right.last().is_some_and(|(c, _)| *c * 10 < before * 6), "{right:?}");
}

#[test]
fn closing_a_tab_sends_kill_then_rm() {
    let r = app(vec![chord(1.0, "t", false), chord(1.5, "w", false)]);
    assert_eq!(r.tabs(), vec!["t-1"]);
    let order: Vec<(&str, &str)> = r.log.iter().filter_map(|l| Some((l["method"].as_str()?, l["params"]["terminal"].as_str().unwrap_or("")))).filter(|(m, _)| *m != "terminal.run" && *m != "terminal.attach").collect();
    assert_eq!(order, vec![("terminal.kill", "t2"), ("terminal.rm", "t2")]);
}

#[test]
fn a_close_from_the_core_closes_the_tab() {
    // The fake ends the terminal like a shell that exits when it reads `exit\r`.
    let r = App::new().fixture("tabs.json").events(vec![chord(0.5, "t", false), typed(0.8, "exit"), enter(1.0)]).end(2.5).run();
    assert_eq!(r.tabs(), vec!["t-1"], "{:?}", r.log);
    assert_eq!(r.data("t2"), "exit\r");
    // The tab's terminal is removed from the core, not left finished.
    assert_eq!(r.calls("terminal.rm").iter().map(|c| c["params"]["terminal"].as_str().unwrap()).collect::<Vec<_>>(), vec!["t2"]);
}

#[test]
fn a_core_that_cannot_run_a_terminal_leaves_a_local_pty() {
    let r = App::new().fixture("tabs-fail.json").events(vec![typed(4.0, "echo $((6*7))"), enter(4.5)]).end(8.0).run();
    assert_eq!(r.calls("terminal.run").len(), 1, "tried once");
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty());
    assert_eq!(r.tabs(), vec!["t-1"], "the tab stays");
    assert!(r.grid().lines().any(|l| l.trim() == "42"), "a local shell answered: {:?}", r.grid());
}

#[test]
fn a_scene_opens_hc_tabs_too() {
    let r = App::new().fixture("tabs-paint.json").scene("three-tabs").end(2.0).run();
    assert_eq!(r.calls("terminal.run").len(), 3);
    assert_eq!(r.tabs().len(), 3);
    assert!(r.grid().contains("Cargo.toml"), "the fixture's first paint: {:?}", r.grid());
}
