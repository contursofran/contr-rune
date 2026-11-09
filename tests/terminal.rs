//! The terminal panel (K3): the grid sits in the card with the target padding (14 top, 20 sides,
//! 12 bottom), there is no header row, and typed text reaches the shell and shows in the grid.
//! Elements: `terminal` (card), `terminal/body` (padded area), `grid`, rows `l<N>`.

use std::sync::OnceLock;

mod common;
use common::*;

/// The launch with no events, shared by the two layout tests.
fn start() -> &'static Run {
    static START: OnceLock<Run> = OnceLock::new();
    START.get_or_init(|| App::new().scene("three-tabs").end(1.0).run())
}

#[test]
fn grid_sits_in_the_card_with_the_target_padding() {
    let t = start();
    let card = t.rect("terminal");
    let grid = t.rect("grid");
    // 14 top, 20 left/right, 12 bottom.
    assert_eq!(grid[0] - card[0], 20.0, "left padding");
    assert_eq!(grid[1] - card[1], 14.0, "top padding");
    assert_eq!((card[0] + card[2]) - (grid[0] + grid[2]), 20.0, "right padding");
    assert_eq!((card[1] + card[3]) - (grid[1] + grid[3]), 12.0, "bottom padding");
    // The first row starts at the grid's top: nothing (no header row) sits above it, and rows are 20 high.
    assert_eq!(t.rect("l0")[1], grid[1]);
    assert_eq!(t.rect("l0")[3], 20.0);
    assert_eq!(t.rect("l1")[1], grid[1] + 20.0);
}

#[test]
fn rows_fill_the_grid_height() {
    let t = start();
    let grid = t.rect("grid");
    let rows = t.elements().iter().filter(|e| e["role"] == "text" && e["key"].as_str().is_some_and(|k| k.starts_with('l') && k[1..].parse::<u32>().is_ok())).count();
    assert_eq!(rows as f64, (grid[3] / 20.0).floor(), "one text row per 20 px of grid height");
}

#[test]
fn typed_text_reaches_the_shell_and_shows_in_the_grid() {
    // A real PTY. `uname` is one word (the `type` event does not keep spaces in order with the next key); its
    // output is a line of its own and cannot be the echoed command. The shell needs real time to start, so
    // the script runs long enough for it.
    let t = App::new().scene("three-tabs").events(vec![typed(4.0, "uname"), key(4.2, "Enter")]).end(8.0).run();
    let shown = t.texts().iter().any(|s| matches!(s.trim(), "Darwin" | "Linux"));
    assert!(shown, "the grid shows the shell's output");
}
