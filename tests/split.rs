//! Split terminal (K7c): the Split button and ⌘D open a second terminal on the right, focus moves
//! by click and ⌘[ / ⌘], typing goes only to the focused pane, the divider resizes (each pane keeps
//! a fifth), and the panes un-split by the button, ⌘W and a shell that exits. Elements: the title
//! bar's `split`; `panes` (the row), `terminal/body` / `terminal/body/1` (the panes), `grid` /
//! `grid/1` (their grids), rows `l<N>` / `p1/l<N>`, `split/divider`; tabs `t-<id>`, sidebar rows
//! `term-<id>`.
//!
//! The terminals are the fake core's (`tabs.json`: echo, and `exit` ends a terminal), so a launch is
//! short and what reaches each terminal is read from the fake's log. One test uses a real PTY: a
//! shell that exits in a pane, because the local PTY is still a product path.

mod common;

use std::sync::OnceLock;

use common::*;
use serde_json::Value;

const FIXTURE: &str = "tabs.json";
const CELL_W: f64 = 13.0 * 0.6;

fn app() -> App { App::new().fixture(FIXTURE) }

fn selected_tab(run: &Run) -> String {
    let tabs: Vec<&Value> = run.tree["elements"].as_array().unwrap().iter().filter(|e| e["role"] == "tab" && e["itemSelected"] == true).collect();
    assert_eq!(tabs.len(), 1, "one selected tab, got {tabs:?}");
    tabs[0]["key"].as_str().unwrap().trim_start_matches("t-").to_owned()
}

fn row_selected(run: &Run, key: &str) -> bool {
    run.find(key).unwrap_or_else(|| panic!("no {key}"))["itemSelected"] == true
}

fn split_open(run: &Run) -> bool { run.find("grid/1").is_some() }

/// The columns of the last RESIZE of `terminal`: what its PTY was told.
fn cols(run: &Run, terminal: &str) -> f64 {
    run.resizes(terminal).last().unwrap_or_else(|| panic!("no resize of {terminal}")).0 as f64
}

/// No events: one panel.
fn base() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| app().run())
}

/// The button pressed once. Shared by the tests that only look at the split's first state.
fn split() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| app().events(vec![tap(0.5, "split")]).run())
}

#[test]
fn the_button_splits_into_two_halves() {
    let (off, t) = (base(), split());
    assert!(!split_open(off) && off.find("split/divider").is_none(), "one panel until split");
    assert!(split_open(t), "a second grid");
    let (card, a, b) = (t.rect("terminal"), t.rect("grid"), t.rect("grid/1"));
    assert!((a[2] - b[2]).abs() <= 2.0, "halves: {} and {}", a[2], b[2]);
    // Half the card, less the 20 point padding each side and the pane's 1 point border.
    assert!((a[2] - (card[2] / 2.0 - 42.0)).abs() <= 3.0, "about half: {} of a {} card", a[2], card[2]);
    assert!(b[0] > a[0] + a[2], "the right grid is to the right");
    // A new terminal in the same project: a tab and a sidebar row, selected, in the right pane.
    assert_eq!(selected_tab(off), "1");
    assert_eq!(selected_tab(t), "2");
    assert!(row_selected(t, "term-2") && !row_selected(t, "term-1"));
    assert!(t.find("t-1").is_some() && t.find("term-1").is_some());
    assert_ne!(t.find("split").unwrap()["fill"], off.find("split").unwrap()["fill"], "the button looks pressed while split");
}

#[test]
fn the_chord_splits_and_unsplits() {
    // Two presses: the first splits (the focus goes to terminal 2), the second takes the panel.
    let t = app().events(vec![chord(0.5, "d", false), chord(1.2, "d", false)]).run();
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "2", "the focused (right) terminal takes the panel");
    assert!(t.find("t-1").is_some(), "the other terminal stays open as a tab");
}

#[test]
fn the_button_unsplits_and_the_focused_pane_takes_the_panel() {
    // Focus the left pane, then un-split: terminal 1 is the whole panel, and its PTY grows to match.
    let t = app().events(vec![tap(0.5, "split"), tap(1.0, "grid"), tap(1.5, "split")]).run();
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "1");
    let (card, g) = (t.rect("terminal"), t.rect("grid"));
    assert_eq!(g[2], card[2] - 40.0, "the card less the 20 point padding each side");
    assert_eq!(cols(&t, "t1"), (g[2] / CELL_W).floor(), "the PTY has the full width");
}

#[test]
fn each_pty_has_its_own_width() {
    let t = split();
    let (a, b) = (t.rect("grid"), t.rect("grid/1"));
    assert_eq!(cols(t, "t1"), (a[2] / CELL_W).floor(), "left cols for a {} wide grid", a[2]);
    assert_eq!(cols(t, "t2"), (b[2] / CELL_W).floor(), "right cols for a {} wide grid", b[2]);
}

#[test]
fn focus_follows_clicks_and_the_bracket_keys() {
    // The first state (the new right terminal focused) is the button test's run.
    assert_eq!(selected_tab(split()), "2");
    let t = app().events(vec![tap(0.5, "split"), tap(1.0, "grid")]).run();
    assert_eq!(selected_tab(&t), "1", "a click on the left pane focuses it; the strip follows");
    assert!(row_selected(&t, "term-1") && !row_selected(&t, "term-2"), "so does the sidebar");
    // ⌘[ moves left, ⌘] right.
    let t = app().events(vec![tap(0.5, "split"), chord(1.0, "[", false), chord(1.5, "]", false)]).run();
    assert_eq!(selected_tab(&t), "2", "⌘[ then ⌘] ends on the right pane");
    assert!(split_open(&t), "moving the focus keeps the split");
    let t = app().events(vec![tap(0.5, "split"), chord(1.0, "[", false)]).run();
    assert_eq!(selected_tab(&t), "1", "⌘[ moves left");
}

#[test]
fn typing_goes_only_to_the_focused_pane() {
    let t = app().events(vec![tap(0.5, "split"), typed(1.0, "one"), tap(1.3, "grid"), typed(1.6, "two")]).run();
    assert_eq!(t.data("t2"), "one", "the right pane was focused first");
    assert_eq!(t.data("t1"), "two");
    let (l, r) = (t.grid_rows("l"), t.grid_rows("p1/l"));
    assert!(r.contains("one") && !r.contains("two"), "right: {r}");
    assert!(l.contains("two") && !l.contains("one"), "left: {l}");
}

#[test]
fn selecting_a_tab_shows_it_in_the_focused_pane() {
    let three = || app().scene("three-tabs");
    // Split from tab 3: left shows 3, right the new tab 4. Tab 1 now fills the focused (right) pane.
    let t = three().events(vec![tap(0.5, "split"), tap(1.0, "t-1")]).run();
    assert!(split_open(&t));
    assert_eq!(selected_tab(&t), "1", "tab 1 now fills the focused (right) pane");
    assert!(row_selected(&t, "term-1") && !row_selected(&t, "term-4"));
    // Selecting the tab of the other pane moves the focus there instead.
    let t = three().events(vec![tap(0.5, "split"), tap(1.0, "t-3")]).run();
    assert_eq!(selected_tab(&t), "3");
    assert!(split_open(&t));
    // A sidebar row does the same as its tab.
    let t = three().events(vec![tap(0.5, "split"), tap(1.0, "term-3")]).run();
    assert_eq!(selected_tab(&t), "3");
}

#[test]
fn dragging_the_divider_resizes_and_keeps_a_fifth() {
    let (row0, a0) = (split().rect("panes"), split().rect("grid"));
    let t = app().events(vec![tap(0.5, "split"), drag(1.0, "split/divider", 200.0)]).run();
    let a1 = t.rect("grid");
    assert!((a1[2] - a0[2] - 200.0).abs() <= 3.0, "the left grid grew by the drag: {} to {}", a0[2], a1[2]);
    assert!(t.rect("grid/1")[2] < a0[2] - 190.0, "the right one shrank");
    // A drag far past the edge stops at a fifth of the panel (the other edge is the unit test
    // `clamp_keeps_each_pane_at_a_fifth`).
    let t = app().events(vec![tap(0.5, "split"), drag(1.0, "split/divider", -2000.0)]).run();
    let a = t.rect("terminal/body");
    assert!((a[2] - row0[2] * 0.2).abs() <= 2.0, "left pane is a fifth: {} of {}", a[2], row0[2]);
}

#[test]
fn closing_the_focused_terminal_unsplits() {
    // ⌘W closes the focused (right) terminal.
    let t = app().events(vec![tap(0.5, "split"), chord(1.0, "w", false)]).run();
    assert!(!split_open(&t));
    assert!(t.find("t-2").is_none() && t.find("t-1").is_some());
    assert_eq!(selected_tab(&t), "1");
}

#[test]
fn a_shell_exiting_in_the_right_pane_unsplits() {
    let t = app().events(vec![tap(0.5, "split"), typed(1.0, "exit"), key(1.3, "Enter")]).run();
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "1");
}

#[test]
fn a_shell_exiting_in_the_left_pane_leaves_the_right_one_as_the_panel() {
    let t = app().events(vec![tap(0.5, "split"), tap(1.0, "grid"), typed(1.3, "exit"), key(1.6, "Enter")]).run();
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "2");
}

/// The local PTY path (hc off): a real shell that exits in the right pane. A real shell races the
/// frame clock, so the script runs long.
#[test]
fn a_real_shell_exiting_in_a_pane_unsplits() {
    let t = App::new().scene("find-text").events(vec![tap(3.0, "split"), typed(4.0, "exit"), key(4.8, "Enter")]).end(20.0).run();
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "1");
}
