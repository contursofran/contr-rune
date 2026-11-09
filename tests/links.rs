//! Links in the terminal (T2): ⌘-click on an http/https/mailto URL in the grid opens it, a plain click
//! does not, and other schemes do nothing. A headless run opens nothing: the link shows in the tree as
//! `terminal/opened-url` (its label). The terminal is the fake core's (`links.json`). URL detection is a
//! unit table in `src/select.rs`.

mod common;

use common::*;
use serde_json::{Value, json};

const CELL_W: f64 = 13.0 * 0.6;
const LINE_H: f64 = 20.0;

fn app() -> App { App::new().fixture("links.json") }

/// A click at column `col` of row `row`, with ⌘ held when `cmd`.
fn click(origin: &[f64], row: f64, col: f64, cmd: bool) -> Vec<Value> {
    let at = json!({ "t": 0.5, "type": "click", "x": origin[0] + col * CELL_W + 2.0, "y": origin[1] + row * LINE_H + 10.0 });
    if cmd { vec![json!({ "t": 0.4, "hold": "cmd" }), at, json!({ "t": 0.9, "hold": "" })] } else { vec![at] }
}

fn opened(run: &Run) -> Option<String> {
    run.find("terminal/opened-url").map(|e| e["label"].as_str().unwrap_or("").to_owned())
}

#[test]
fn cmd_click_opens_a_url_and_a_plain_click_does_not() {
    let o = app().run().rect("l0");
    let t = app().events(click(&o, 0.0, 10.0, true)).run();
    assert_eq!(opened(&t).as_deref(), Some("https://example.com/docs"));
    let t = app().events(click(&o, 1.0, 10.0, true)).run();
    assert_eq!(opened(&t).as_deref(), Some("mailto:me@example.com"));
    let t = app().events(click(&o, 0.0, 10.0, false)).run();
    assert_eq!(opened(&t), None);
}

#[test]
fn cmd_click_off_a_url_or_on_another_scheme_does_nothing() {
    let o = app().run().rect("l0");
    for (row, col) in [(0.0, 1.0), (2.0, 10.0), (3.0, 10.0)] {
        let t = app().events(click(&o, row, col, true)).run();
        assert_eq!(opened(&t), None, "row {row} col {col}");
    }
}

/// The pointer moved onto column `col` of row `row` at `t`.
fn move_to(t: f64, o: &[f64], row: f64, col: f64) -> Value {
    json!({ "t": t, "type": "move", "x": o[0] + col * CELL_W + 2.0, "y": o[1] + row * LINE_H + 10.0 })
}

fn hovered(run: &Run) -> Option<String> {
    run.find("terminal/link-hover").map(|e| e["label"].as_str().unwrap_or("").to_owned())
}

#[test]
fn a_link_under_the_pointer_shows_while_cmd_is_held_and_goes_with_it_or_with_the_pointer() {
    let o = app().run().rect("l0");
    let cmd = |t, on: bool| json!({ "t": t, "hold": if on { "cmd" } else { "" } });
    // ⌘ down, then the pointer onto the URL.
    let t = app().events(vec![cmd(0.3, true), move_to(0.5, &o, 0.0, 10.0)]).run();
    assert_eq!(hovered(&t).as_deref(), Some("https://example.com/docs"));
    // The pointer first, then ⌘.
    let t = app().events(vec![move_to(0.3, &o, 0.0, 10.0), cmd(0.5, true)]).run();
    assert_eq!(hovered(&t).as_deref(), Some("https://example.com/docs"));
    // ⌘ released, the pointer off the URL, a scheme that does not open, or no ⌘ at all: nothing.
    for events in [
        vec![cmd(0.3, true), move_to(0.5, &o, 0.0, 10.0), cmd(0.7, false)],
        vec![cmd(0.3, true), move_to(0.5, &o, 0.0, 10.0), move_to(0.7, &o, 0.0, 1.0)],
        vec![cmd(0.3, true), move_to(0.5, &o, 3.0, 10.0)],
        vec![move_to(0.5, &o, 0.0, 10.0)],
    ] {
        let t = app().events(events.clone()).run();
        assert_eq!(hovered(&t), None, "{events:?}");
    }
}
