//! Selecting and copying terminal text. The selection is alacritty's own (`Term::selection`), so it
//! follows the text into scrollback; this module turns pointer positions into cells, runs the
//! gesture (drag selects, double-click selects a word) and reads the selected text for ⌘C.
//! A selection goes when output arrives in its terminal or when a key is sent to the shell.

use std::sync::atomic::Ordering;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Term;
use rune::{Drag, Phase};

use crate::theme::{CELL_W, LINE_H};
use crate::App;

/// Which terminal holds the selection and the content generation it was made at.
#[derive(Default)]
pub struct Select {
    owner: Option<(u64, u64)>,
}

/// The cell under `(x, y)` (points inside the grid area) and the side of it the pointer is on.
/// Positions outside the grid clamp to its edge. `offset` is how far the view is scrolled back.
fn locate(x: f32, y: f32, cols: usize, rows: usize, offset: usize) -> (Point, Side) {
    let fx = (x / CELL_W).clamp(0.0, cols as f32 - 0.001);
    let col = fx.floor() as usize;
    let row = (y / LINE_H).floor().clamp(0.0, rows as f32 - 1.0) as i32;
    let side = if fx.fract() < 0.5 { Side::Left } else { Side::Right };
    (Point::new(Line(row - offset as i32), Column(col)), side)
}

fn at<T>(term: &Term<T>, x: f32, y: f32) -> (Point, Side) {
    locate(x, y, term.columns(), term.screen_lines(), term.grid().display_offset())
}

/// The selected text, `None` when nothing (or only blank cells) is selected.
pub fn text<T>(term: &Term<T>) -> Option<String> {
    term.selection_to_string().filter(|t| !t.is_empty())
}

/// The pointer gesture of pane `idx`'s grid: a press clears the selection (a second press selects
/// the word), a drag selects from where it began.
pub fn drag(s: &mut App, idx: usize, d: &Drag) {
    let Some(t) = s.pane_tab(idx).and_then(|id| s.tabs.iter().find(|t| t.session.id() == id)) else { return };
    let id = t.session.id();
    let mut term = t.session.term().lock().unwrap();
    let generation = t.session.generation().load(Ordering::Acquire);
    match d.phase {
        Phase::Down => {
            term.selection = None;
            if d.clicks == 2 {
                let (p, side) = at(&term, d.x, d.y);
                term.selection = Some(Selection::new(SelectionType::Semantic, p, side));
            }
        }
        Phase::Move if d.clicks == 1 => {
            if term.selection.is_none() {
                let (p, side) = at(&term, d.start_x, d.start_y);
                term.selection = Some(Selection::new(SelectionType::Simple, p, side));
            }
            let (p, side) = at(&term, d.x, d.y);
            if let Some(sel) = term.selection.as_mut() { sel.update(p, side); }
        }
        _ => return,
    }
    s.select.owner = term.selection.is_some().then_some((id, generation));
}

/// Drop the selection of terminal `id` (a key went to the shell).
pub fn clear(s: &App, id: u64) {
    if let Some(t) = s.tabs.iter().find(|t| t.session.id() == id) { t.session.term().lock().unwrap().selection = None; }
}

/// Drop the selection when its terminal printed since it was made. True when it was dropped.
pub fn sweep(s: &mut App) -> bool {
    let Some((id, generation)) = s.select.owner else { return false };
    let current = s.tabs.iter().find(|t| t.session.id() == id).map(|t| t.session.generation().load(Ordering::Acquire));
    if current == Some(generation) { return false; }
    clear(s, id);
    s.select.owner = None;
    true
}

/// The text ⌘C copies: the selection's, when there is one.
pub fn copied(s: &App) -> Option<String> {
    let (id, _) = s.select.owner?;
    let t = s.tabs.iter().find(|t| t.session.id() == id)?;
    text(&t.session.term().lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    struct Dim;
    impl Dimensions for Dim {
        fn total_lines(&self) -> usize { 3 }
        fn screen_lines(&self) -> usize { 3 }
        fn columns(&self) -> usize { 20 }
    }

    #[test]
    fn text_is_read_from_scrollback_and_empty_selections_read_as_none() {
        let mut term = Term::new(Config { scrolling_history: 100, ..Config::default() }, &Dim, VoidListener);
        let mut p: Processor = Processor::new();
        p.advance(&mut term, b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        term.scroll_display(Scroll::Delta(2));
        // The view now shows "two" at its top row, a scrollback line of the grid.
        let (start, side) = at(&term, 1.0, 1.0);
        let (end, end_side) = at(&term, CELL_W * 2.5, LINE_H * 1.5);
        assert!(start.line.0 < 0, "{start:?}");
        let mut sel = Selection::new(SelectionType::Simple, start, side);
        sel.update(end, end_side);
        term.selection = Some(sel);
        assert_eq!(text(&term).as_deref(), Some("two\nthr"));
        // A click without a drag leaves a selection of nothing.
        term.selection = Some(Selection::new(SelectionType::Simple, start, side));
        assert_eq!(text(&term), None);
    }

    #[test]
    fn pointer_positions_become_cells_and_clamp_to_the_grid() {
        let cell = |x, y, off| { let (p, s) = locate(x, y, 80, 24, off); (p.line.0, p.column.0, s) };
        assert_eq!(cell(0.0, 0.0, 0), (0, 0, Side::Left));
        assert_eq!(cell(CELL_W * 3.0 + CELL_W * 0.7, LINE_H * 2.5, 0), (2, 3, Side::Right));
        // Scrolled back: the top row is that many lines above the screen.
        assert_eq!(cell(1.0, 1.0, 5).0, -5);
        // Outside: the last cell, the first column.
        assert_eq!(cell(9999.0, 9999.0, 0).0, 23);
        assert_eq!(cell(9999.0, 0.0, 0).1, 79);
        assert_eq!(cell(-30.0, -30.0, 0), (0, 0, Side::Left));
    }
}
