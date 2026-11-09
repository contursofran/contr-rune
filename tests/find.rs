//! Search in the terminal (K7b): the Search button and ⌘F open the find bar, typing counts the
//! matches ("1 of 4"), Return / Shift+Return / ⌘G step with wrap, esc closes it, and keys typed
//! in the bar never reach the shell. The terminal is the fake core's (`find.json`): its first paint
//! has four lines that hold "error" in different cases, and it echoes what is typed. Elements:
//! `find`, `find/input`, `find/count`, `find/prev`, `find/next`, `find/close`, the title bar's `search`.
//! Matching, case and wrap are unit tests of `find`.

mod common;

use std::sync::OnceLock;

use common::*;

fn app() -> App { App::new().fixture("find.json") }

fn bar_open(run: &Run) -> bool {
    run.find("find").is_some()
}

fn count(run: &Run) -> String {
    run.find("find/count").expect("the find bar is open")["text"].as_str().unwrap_or("").to_owned()
}

/// The bar opened by the button and "error" typed in it. Shared by the tests that only look at it.
fn error_query() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| app().events(vec![tap(0.5, "search"), typed(1.0, "error")]).run())
}

/// After a grid click the shell gets the keys while the bar stays: "error" typed in the grid.
fn grid_click() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| app().events(vec![tap(0.5, "search"), typed(1.0, "error"), tap(1.3, "grid"), typed(1.6, "error")]).run())
}

#[test]
fn the_bar_is_closed_until_asked_for() {
    let t = app().run();
    assert!(!bar_open(&t));
    assert!(t.find("search").is_some(), "the title bar has the Search button");
}

#[test]
fn the_search_button_opens_the_bar_at_the_top_right_of_the_terminal() {
    let t = error_query();
    assert!(bar_open(t));
    for k in ["find/input", "find/count", "find/prev", "find/next", "find/close"] {
        assert!(t.find(k).is_some(), "{k}");
    }
    let (bar, card) = (t.rect("find"), t.rect("terminal"));
    assert_eq!(bar[1] - card[1], 10.0, "10 below the card's top");
    assert_eq!((card[0] + card[2]) - (bar[0] + bar[2]), 12.0, "12 from the card's right edge");
}

#[test]
fn typing_counts_the_matches_in_any_case() {
    assert_eq!(count(error_query()), "1 of 4", "error, ERROR, Error and error");
}

#[test]
fn next_and_previous_move_the_current_match() {
    // Two steps forward and one back: 1, 2, 3, 2.
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), key(1.3, "Enter"), key(1.5, "Enter"), shift_key(1.7, "Enter")]).run();
    assert_eq!(count(&t), "2 of 4");
}

#[test]
fn the_chord_opens_the_bar_and_steps() {
    // ⌘F opens it; ⌘G steps forward twice and ⇧⌘G back once.
    let t = app().events(vec![chord(0.5, "f", false), typed(1.0, "error"), chord(1.3, "g", false), chord(1.5, "g", false), chord(1.7, "g", true)]).run();
    assert!(bar_open(&t));
    assert_eq!(count(&t), "2 of 4");
}

#[test]
fn the_next_and_previous_buttons_step_too() {
    let ev = vec![tap(0.5, "search"), typed(1.0, "error"), tap(1.3, "find/next"), tap(1.5, "find/next"), tap(1.7, "find/prev")];
    assert_eq!(count(&app().events(ev).run()), "2 of 4");
}

#[test]
fn esc_and_the_close_button_close_the_bar() {
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), key(1.3, "Escape")]).run();
    assert!(!bar_open(&t), "esc closes it");
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), tap(1.3, "find/close")]).run();
    assert!(!bar_open(&t), "the close button closes it");
}

#[test]
fn the_bar_reopens_and_keeps_working_after_esc() {
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), key(1.2, "Escape"), tap(1.4, "search"), key(1.6, "Enter")]).run();
    assert!(bar_open(&t));
    assert_eq!(count(&t), "2 of 4", "the old query is back (1 of 4) and Return steps to the second");
}

#[test]
fn keys_typed_in_the_bar_do_not_reach_the_shell() {
    // "zzqword" typed in the bar, then esc, then "uname" typed in the terminal: only the latter is DATA.
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "zzqword"), key(1.2, "Enter"), key(1.4, "Escape"), typed(1.6, "uname"), key(1.8, "Enter")]).run();
    assert_eq!(t.data("t1"), "uname\r", "the bar kept its keys; after esc the shell gets them");
    assert!(!t.grid_rows("l").contains("zzqword"), "no row of the grid shows it");
}

#[test]
fn another_tab_starts_without_the_bar() {
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "a"), tap(1.3, "new-tab")]).run();
    assert!(!bar_open(&t), "a new tab closes the bar and its highlights");
}

#[test]
fn a_click_on_the_grid_gives_the_keys_back_to_the_shell_and_keeps_the_bar() {
    let t = grid_click();
    assert_eq!(t.data("t1"), "error", "the shell got the keys");
    assert!(bar_open(t), "the bar stays open");
    // The query is still "error": had the input taken the keys it would read "errorerror" (no results).
    assert_ne!(count(t), "No results", "the input did not get them");
}

#[test]
fn new_output_updates_the_count() {
    // The echo of "error" in the grid is new output: 4 matches on the first paint plus this one,
    // found without touching the query.
    assert_eq!(count(grid_click()), "1 of 5");
}

#[test]
fn after_a_grid_click_the_input_takes_the_keys_again_by_click_or_chord() {
    // The input holds "error"; typing there appends.
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), tap(1.2, "grid"), tap(1.4, "find/input"), typed(1.6, "zzq")]).run();
    assert_eq!(count(&t), "No results", "a click on the input focuses it");
    // The chord selects the old query, so typing replaces it.
    let t = app().events(vec![tap(0.5, "search"), typed(1.0, "error"), tap(1.2, "grid"), chord(1.4, "f", false), typed(1.6, "ERROR")]).run();
    assert_eq!(count(&t), "1 of 4", "the chord focuses the input and selects the old query");
}
