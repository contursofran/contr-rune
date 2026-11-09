//! The title bar's tab strip (K2): `--script` drives clicks and keys against the `three-tabs`
//! scene (three real shells) and `--tree` reads the result. Tab elements are `t-<id>`, the
//! close button is `t-<id>/.../close`, the `+` button is `new-tab`.

use std::sync::OnceLock;

use serde_json::{Value, json};

mod common;
use common::*;

fn run(events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").events(events).end(1.0).run()
}

/// The launch with no events (the start state), shared by the tests that only read it.
fn start() -> &'static Run {
    static START: OnceLock<Run> = OnceLock::new();
    START.get_or_init(|| run(vec![]))
}

fn ctrl(t: f64, name: &str, shift: bool) -> Value {
    json!({ "t": t, "type": "key", "name": name, "ctrl": true, "shift": shift })
}

#[test]
fn starts_with_three_tabs_and_the_last_selected() {
    let t = start();
    assert_eq!(t.tabs(), ["t-1", "t-2", "t-3"]);
    assert_eq!(t.active_tab(), "t-3");
    // The same bounds as `kit_preview --tree`: 180 x 28 tabs 6 apart from x 342, then `+`.
    assert_eq!(t.rect("t-1"), [342.0, 5.0, 180.0, 28.0]);
    assert_eq!(t.rect("t-2"), [528.0, 5.0, 180.0, 28.0]);
    assert_eq!(t.rect("t-3"), [714.0, 5.0, 180.0, 28.0]);
    assert_eq!(t.rect("top-list"), [342.0, 5.0, 552.0, 28.0]);
}

#[test]
fn click_selects_a_tab() {
    let t = run(vec![tap(0.5, "t-1")]);
    assert_eq!(t.active_tab(), "t-1");
    assert_eq!(t.tabs().len(), 3);
}

#[test]
fn close_removes_the_tab() {
    // The close button sits under the tab; the strip keys it `close`.
    let close = start().child("t-2", "close").expect("close button on t-2");
    let r = &close["rect"];
    let (x, y) = (r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0, r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0);
    // Hover first: the close button only takes the click once the tab shows it.
    let t = run(vec![hover(0.3, "t-2"), json!({ "t": 0.6, "type": "tap", "x": x, "y": y })]);
    assert_eq!(t.tabs(), ["t-1", "t-3"]);
}

#[test]
fn plus_adds_a_tab_and_selects_it() {
    let t = run(vec![tap(0.5, "new-tab")]);
    assert_eq!(t.tabs(), ["t-1", "t-2", "t-3", "t-4"]);
    assert_eq!(t.active_tab(), "t-4");
}

#[test]
fn chords_open_and_close_tabs() {
    let t = run(vec![chord(0.5, "t", false)]);
    assert_eq!(t.tabs(), ["t-1", "t-2", "t-3", "t-4"]);
    assert_eq!(t.active_tab(), "t-4");
    let t = run(vec![chord(0.5, "w", false)]);
    assert_eq!(t.tabs(), ["t-1", "t-2"]);
    assert_eq!(t.active_tab(), "t-2");
}

#[test]
fn ctrl_tab_cycles_both_ways() {
    // Forward from the last tab wraps to the first (3 -> 1), forward again (-> 2), back (-> 1).
    // A wrong direction on any key ends on tab 2.
    let t = run(vec![ctrl(0.3, "Tab", false), ctrl(0.5, "Tab", false), ctrl(0.7, "Tab", true)]);
    assert_eq!(t.active_tab(), "t-1");
}

#[test]
fn ctrl_page_keys_cycle() {
    let t = run(vec![ctrl(0.3, "PageDown", false), ctrl(0.5, "PageDown", false), ctrl(0.7, "PageUp", false)]);
    assert_eq!(t.active_tab(), "t-1");
}

#[test]
fn dragging_a_tab_reorders_the_strip() {
    let t = run(vec![json!({ "t": 0.5, "type": "drag", "key": "t-1", "dx": 200.0, "dy": 0.0, "dur": 0.6 })]);
    // The strip order is by position, not by tree order.
    let mut by_x: Vec<(f64, String)> = t.tabs().into_iter().map(|k| (t.rect(&k)[0], k)).collect();
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(by_x.into_iter().map(|t| t.1).collect::<Vec<_>>(), ["t-2", "t-1", "t-3"]);
}
