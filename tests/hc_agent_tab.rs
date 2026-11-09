//! Selecting an agent shows its terminal (H8), against the in-process fake core (`CONTR_HC_FAKE`, the
//! fixture `tests/fixtures/hc/agents.json` with screens added, log in `CONTR_HC_FAKE_LOG`). The
//! orchestrator t3 is a tab of the shared set (T4) from the start: `t-1` (framed, with history); the
//! plain terminal t20 is restored as `t-2`. Tapping `agent/t3` selects its tab; a worker's tab is added
//! when it is selected (`t-3`, between them). Closing an agent's tab only detaches (a stream CLOSE, no
//! `terminal.kill` or `terminal.rm`); closing a plain tab kills and removes it. No test dials a real hc.

mod common;

use std::sync::OnceLock;

use common::*;
use serde_json::{Value, json};

fn run(events: Vec<Value>) -> Run {
    App::new()
        .project("contr")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx["screens"] = json!({"t3": "orchestrator-screen\r\n", "t17": "builder-screen\r\n", "t20": "plain-screen\r\n"});
            fx
        })
        .events(events)
        .end(1.5)
        .run()
}

/// The launch that taps `agent/t3`: shared by the tests that read it.
fn tapped_agent() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(vec![tap(0.5, "agent/t3")]))
}

#[test]
fn the_orchestrator_is_a_tab_from_the_start_and_tapping_it_shows_its_screen() {
    let r = tapped_agent();
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "the orchestrator, then the plain tab: {:?}", r.log);
    assert_eq!(r.active_tab(), "t-1");
    assert!(r.grid().contains("orchestrator-screen"), "{:?}", r.grid());
    assert!(r.selected("agent/t3"), "the sidebar follows");
    let a = r.attaches("t3");
    assert_eq!(a.len(), 1, "attached once, not again by the tap: {:?}", r.log);
    assert_eq!(a[0]["history"], 1000);
    assert!(r.calls("terminal.run").is_empty(), "an agent's terminal is attached, never run: {:?}", r.log);
}

#[test]
fn the_tab_shows_the_agent_title_and_state_glyph_not_the_prompt_icon() {
    let r = tapped_agent();
    let tab = r.find("t-1").unwrap();
    assert!(tab.to_string().contains("Plan H1"), "the agent's title labels the tab: {tab}");
    let plain = r.find("t-2").unwrap();
    assert!(!plain.to_string().contains("Plan H1"));
    // The glyph (key `glyph`, label = state word) is in the tab as well as in the sidebar row.
    let glyphs: Vec<&Value> = r.elements().iter().filter(|e| e["key"] == "glyph" && !e["path"].as_str().unwrap_or("").contains("/agent/")).collect();
    assert_eq!(glyphs.len(), 1, "one glyph outside the sidebar rows, in the agent's tab: {glyphs:?}");
    assert_eq!(glyphs[0]["label"], "waiting", "the orchestrator state, as in its row");
    assert!(glyphs[0]["path"].as_str().unwrap().contains("t-1"), "{}", glyphs[0]);
}

#[test]
fn tapping_the_same_agent_again_selects_the_tab_instead_of_opening_another() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.8, "t-2"), tap(1.1, "agent/t3")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "no second tab");
    assert_eq!(r.attaches("t3").len(), 1);
    assert_eq!(r.active_tab(), "t-1");
    assert!(r.selected("agent/t3"));
}

#[test]
fn the_tab_strip_and_the_sidebar_follow_the_selection_both_ways() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.8, "t-2")]);
    assert_eq!(r.active_tab(), "t-2");
    assert!(!r.selected("agent/t3"), "a plain tab clears the agent selection");
    assert!(r.grid().contains("plain-screen"), "{:?}", r.grid());
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.8, "agent/t17")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-3", "t-2"], "the worker joins between the Claude session and the plain tab");
    assert_eq!(r.active_tab(), "t-3");
    assert!(r.selected("agent/t17") && !r.selected("agent/t3"), "one agent at a time");
    assert!(r.grid().contains("builder-screen"), "{:?}", r.grid());
    let r = run(vec![tap(0.5, "agent/t17"), tap(0.8, "t-1")]);
    assert!(r.selected("agent/t3") && !r.selected("agent/t17"), "selecting an agent's tab selects its row");
}

#[test]
fn closing_an_agent_tab_only_detaches_and_it_stays_out_until_selected() {
    let r = run(vec![tap(0.5, "agent/t3"), chord(1.0, "w", false)]);
    assert_eq!(r.tabs(), vec!["t-2"], "the tab is gone and stays gone");
    assert_eq!(r.closes("t3"), 1, "a stream CLOSE: {:?}", r.log);
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty(), "the agent lives on: {:?}", r.log);
    assert!(!r.selected("agent/t3"));
    let r = run(vec![tap(0.5, "agent/t3"), chord(0.8, "w", false), tap(1.1, "agent/t3")]);
    assert_eq!(r.tabs(), vec!["t-3", "t-2"], "selecting it in the sidebar brings the tab back: {:?}", r.log);
    assert_eq!(r.active_tab(), "t-3");
}

#[test]
fn an_agent_tab_has_no_terminal_row_in_the_sidebar() {
    let r = tapped_agent();
    let rows: Vec<&str> = r.elements().iter().filter_map(|e| e["key"].as_str()).filter(|k| k.starts_with("term-")).collect();
    assert_eq!(rows, vec!["term-2"], "only the plain tab has a terminal row");
    assert!(r.selected("agent/t3"));
}
