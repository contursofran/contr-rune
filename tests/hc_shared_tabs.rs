//! The shared tab set (T4, D190/D191) against the in-process fake core and the fixture
//! `tests/fixtures/hc/agents.json`: the orchestrator t3 is tab `t-1`, the restored plain terminal t20
//! is `t-2`; a worker's tab (t17 is `t-3`, t18 `t-4`) joins between them while it is the kept worker.
//! Claude sessions come first, the set does not change with the selection, a plain tab keeps the last
//! worker, a Claude session's tab drops it, and a session that does not orchestrate has a grey mark.

mod common;

use common::*;
use serde_json::{Value, json};

fn fixture(edit: impl FnOnce(&mut Vec<Value>) + 'static) -> impl FnOnce(&Dirs) -> Value {
    move |d| {
        let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
        for t in fx["terminals"].as_array_mut().unwrap() {
            t["cwd"] = json!(d.proj);
        }
        edit(fx["terminals"].as_array_mut().unwrap());
        fx
    }
}

fn run_with(edit: impl FnOnce(&mut Vec<Value>) + 'static, events: Vec<Value>) -> Run {
    App::new().project("contr").fixture_with(fixture(edit)).events(events).end(2.0).run()
}

fn run(events: Vec<Value>) -> Run {
    run_with(|_| {}, events)
}

/// The colour of the glyph under `scope` (a path part such as `/t-1/` or `/agent/t3/`), as RGB.
fn glyph_color(r: &Run, scope: &str) -> (u8, u8, u8) {
    let e = r.elements().iter().find(|e| e["key"] == "glyph" && e["path"].as_str().is_some_and(|p| p.contains(scope))).unwrap_or_else(|| panic!("no glyph in {scope}"));
    let hex = e["color"].as_str().unwrap_or_else(|| panic!("no colour: {e}")).trim_start_matches('#').to_owned();
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    (c(0), c(2), c(4))
}

fn grey((r, g, b): (u8, u8, u8)) -> bool {
    r.abs_diff(g) < 16 && g.abs_diff(b) < 16
}

#[test]
fn the_set_is_claude_sessions_then_plain_terminals_and_the_selection_does_not_change_it() {
    for events in [vec![], vec![tap(0.5, "t-2")], vec![tap(0.5, "t-2"), tap(0.9, "t-1")], vec![tap(0.5, "agent/t3")]] {
        let r = run(events);
        assert_eq!(r.tabs(), vec!["t-1", "t-2"], "{:?}", r.log);
    }
    assert_eq!(run(vec![]).attaches("t3").len(), 1, "the orchestrator is attached once, at launch");
}

#[test]
fn a_plain_terminal_running_claude_is_a_session_and_comes_before_the_plain_ones() {
    // t21 is a plain shell older than t20; t20 runs Claude Code, so it goes before t21 (and has a row-less tab).
    let edit = |ts: &mut Vec<Value>| {
        let mut shell = ts.iter().find(|t| t["address"] == "t20").unwrap().clone();
        shell["address"] = json!("t21");
        shell["name"] = json!("term-00002");
        shell["created_ms"] = json!(500);
        ts.push(shell);
        let t20 = ts.iter_mut().find(|t| t["address"] == "t20").unwrap();
        t20["foreground"] = json!({"pid": 204, "name": "claude", "argv": [], "cwd": "/tmp/contr"});
    };
    let r = run_with(edit, vec![]);
    // Restore opens t21 (`t-2`) before t20 (`t-3`); the set puts the Claude one first among the plain ones.
    assert_eq!(r.tabs(), vec!["t-1", "t-3", "t-2"], "{:?}", r.log);
    assert!(grey(glyph_color(&r, "/t-3/")), "the Claude mark of a plain terminal is grey: {:?}", glyph_color(&r, "/t-3/"));
    assert!(!r.find("t-2").unwrap().to_string().contains("\"glyph\""), "a plain shell keeps the prompt icon");
}

#[test]
fn selecting_a_worker_adds_its_tab_and_another_worker_takes_its_place() {
    let r = run(vec![tap(0.5, "agent/t17")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-3", "t-2"], "the worker sits after the Claude session: {:?}", r.log);
    assert_eq!(r.active_tab(), "t-3");
    let r = run(vec![tap(0.5, "agent/t17"), tap(1.0, "agent/t18")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-4", "t-2"], "{:?}", r.log);
    assert_eq!(r.closes("t17"), 1, "the old worker only detaches: {:?}", r.log);
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty(), "{:?}", r.log);
}

#[test]
fn a_plain_tab_keeps_the_last_worker_and_a_claude_session_drops_it() {
    let r = run(vec![tap(0.5, "agent/t17"), tap(1.0, "t-2")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-3", "t-2"], "the plain tab keeps the worker: {:?}", r.log);
    assert_eq!(r.active_tab(), "t-2");
    assert_eq!(r.closes("t17"), 0);
    assert!(!r.selected("agent/t17"), "kept in the strip, not selected in the sidebar");
    let r = run(vec![tap(0.5, "agent/t17"), tap(1.0, "t-1")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "a Claude session's tab drops the worker: {:?}", r.log);
    assert_eq!(r.closes("t17"), 1);
}

#[test]
fn an_orchestrator_with_no_work_has_a_grey_mark_and_one_with_workers_keeps_the_claude_hue() {
    let r = run(vec![]);
    let hue = glyph_color(&r, "/t-1/");
    assert!(!grey(hue), "an orchestrating session keeps the Claude hue: {hue:?}");
    assert_eq!(glyph_color(&r, "/agent/t3/"), hue, "the row and the tab agree");
    // Without its workers t3 neither ran /contr:orchestrate nor has a live worker.
    let r = run_with(|ts| ts.retain(|t| t["address"] == "t3" || t["address"] == "t20"), vec![]);
    assert!(grey(glyph_color(&r, "/t-1/")), "{:?}", glyph_color(&r, "/t-1/"));
    assert!(grey(glyph_color(&r, "/agent/t3/")), "the row is grey too");
    // The `contr.orchestrating` label (the Stop hook sets it) brings the hue back, before any worker.
    let r = run_with(
        |ts| {
            ts.retain(|t| t["address"] == "t3" || t["address"] == "t20");
            ts[0]["labels"]["contr.orchestrating"] = json!("1");
        },
        vec![],
    );
    assert!(!grey(glyph_color(&r, "/t-1/")));
}
