//! Select and copy terminal text (T1): a drag on the grid selects, a double press selects a word, ⌘C
//! copies, and a key sent to the shell clears the selection. Elements: `terminal/selection` (its
//! label is the selected text; `/1` in the second pane) and the rows `l<N>`. The terminal is the fake
//! core's (`find.json`): its first paint has six lines of text and it echoes what is typed, so what ⌘C
//! copied is read back by pasting it (⌘V) and looking at what the shell was sent. Mapping pointer to
//! cell is a unit test of `select`.

mod common;

use common::*;
use serde_json::{Value, json};

const CELL_W: f64 = 13.0 * 0.6;
const LINE_H: f64 = 20.0;

fn app() -> App { App::new().fixture("find.json") }

/// The top left of the first row: the corner of cell (0, 0).
fn origin() -> (f64, f64) {
    let r = app().run().rect("l0");
    (r[0], r[1])
}

/// A drag from `col` of row `row` over `cells` columns and `rows` rows.
fn drag_cells(t: f64, (x, y): (f64, f64), row: f64, col: f64, cells: f64, rows: f64) -> Value {
    json!({ "t": t, "type": "drag", "x": x + col * CELL_W + 1.0, "y": y + row * LINE_H + 10.0, "dx": cells * CELL_W, "dy": rows * LINE_H, "dur": 0.1 })
}

fn double_click(t: f64, (x, y): (f64, f64), row: f64, col: f64) -> Value {
    json!({ "t": t, "type": "double-click", "x": x + col * CELL_W + 3.0, "y": y + row * LINE_H + 10.0 })
}

fn cmd(t: f64, name: &str) -> Value { json!({ "t": t, "type": "key", "name": name, "cmd": true }) }

fn selected(run: &Run) -> Option<String> {
    run.find("terminal/selection").map(|e| e["label"].as_str().unwrap_or("").to_owned())
}

#[test]
fn a_drag_selects_the_text_it_covers() {
    let o = origin();
    // "error" of "error: unable to resolve import": columns 0 to 4.
    let t = app().events(vec![drag_cells(0.5, o, 0.0, 0.0, 5.0, 0.0)]).run();
    assert_eq!(selected(&t).as_deref(), Some("error"));
}

#[test]
fn a_drag_over_rows_selects_across_the_line_break() {
    let o = origin();
    let t = app().events(vec![drag_cells(0.5, o, 0.0, 25.0, 8.0, 1.0)]).run();
    let text = selected(&t).expect("a selection");
    assert!(text.starts_with("import\nERROR: e"), "{text:?}");
}

#[test]
fn a_double_press_selects_the_word_under_it() {
    let o = origin();
    // "ERROR: expected semicolon": column 10 is in "expected".
    let t = app().events(vec![double_click(0.5, o, 1.0, 10.0)]).run();
    assert_eq!(selected(&t).as_deref(), Some("expected"));
}

#[test]
fn a_plain_click_selects_nothing_and_drops_the_selection() {
    let o = origin();
    let t = app().events(vec![drag_cells(0.5, o, 0.0, 0.0, 5.0, 0.0), tap(1.0, "grid")]).run();
    assert_eq!(selected(&t), None);
}

#[test]
fn cmd_c_keeps_the_selection_and_sends_nothing_to_the_shell() {
    // The headless run has no readable clipboard: what is copied is `select::text`, a unit test.
    let o = origin();
    let t = app().events(vec![drag_cells(0.5, o, 0.0, 0.0, 5.0, 0.0), cmd(1.0, "c")]).run();
    assert_eq!(selected(&t).as_deref(), Some("error"));
    assert_eq!(t.data("t1"), "");
}

#[test]
fn typing_clears_the_selection_and_still_reaches_the_shell() {
    let o = origin();
    let t = app().events(vec![drag_cells(0.5, o, 0.0, 0.0, 5.0, 0.0), typed(1.0, "x")]).run();
    assert_eq!(selected(&t), None);
    assert_eq!(t.data("t1"), "x");
}

/// Down on row `row`, move to `y` (a grid-relative offset of the first row's top) and hold there until `up`.
fn hold_at(origin: (f64, f64), row: f64, y: f64, up: f64) -> Vec<Value> {
    let (x, y0) = origin;
    vec![
        json!({ "t": 0.5, "type": "down", "x": x + 1.0, "y": y0 + row * LINE_H + 10.0 }),
        json!({ "t": 0.6, "type": "move", "x": x + 40.0, "y": y0 + y }),
        json!({ "t": up, "type": "up", "x": x + 40.0, "y": y0 + y }),
    ]
}

fn lines(run: &Run) -> usize { selected(run).map_or(0, |t| t.lines().count()) }

#[test]
fn a_triple_press_selects_the_whole_line() {
    let o = origin();
    let t = app().events(vec![json!({ "t": 0.5, "type": "triple-click", "x": o.0 + 10.0 * CELL_W + 3.0, "y": o.1 + 1.0 * LINE_H + 10.0 })]).run();
    assert_eq!(selected(&t).as_deref(), Some("ERROR: expected semicolon\n"));
}

fn scrolled() -> App { App::new().fixture("scroll.json") }

#[test]
fn a_drag_held_above_the_grid_scrolls_back_and_keeps_selecting() {
    let o = scrolled().run().rect("l0");
    let o = (o[0], o[1]);
    let short = scrolled().events(hold_at(o, 3.0, -30.0, 0.8)).run();
    let long = scrolled().events(hold_at(o, 3.0, -30.0, 1.6)).run();
    // Row 3 is the anchor; the selection grows by a line on each tick the pointer stays above.
    assert!(lines(&short) > 4, "{}", lines(&short));
    assert!(lines(&long) > lines(&short) + 3, "{} vs {}", lines(&long), lines(&short));
}

#[test]
fn the_scrolling_stops_when_the_pointer_comes_back_or_the_button_goes_up() {
    let o = scrolled().run().rect("l0");
    let o = (o[0], o[1]);
    // Back inside at 1.0: nothing grows between 1.1 and 1.6.
    let mut back = hold_at(o, 3.0, -30.0, 1.5);
    back[2]["y"] = json!(o.1 + 2.0 * LINE_H);
    back.insert(2, json!({ "t": 1.0, "type": "move", "x": o.0 + 40.0, "y": o.1 + 2.0 * LINE_H }));
    let a = scrolled().events(back.clone()).end(1.1).run();
    let b = scrolled().events(back.clone()).end(1.7).run();
    assert_eq!(lines(&a), lines(&b));
    // Released at 1.0: the same.
    let c = scrolled().events(hold_at(o, 3.0, -30.0, 1.0)).end(1.1).run();
    let d = scrolled().events(hold_at(o, 3.0, -30.0, 1.0)).end(1.7).run();
    assert_eq!(lines(&c), lines(&d));
}
