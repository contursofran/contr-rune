//! Selecting and copying terminal text. The selection is alacritty's own (`Term::selection`), so it
//! follows the text into scrollback; this module turns pointer positions into cells, runs the
//! gesture (drag selects, double-click selects a word) and reads the selected text for ⌘C.
//! A selection goes when output arrives in its terminal or when a key is sent to the shell.

use std::sync::atomic::Ordering;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use rune::{Drag, Phase, Tx};

use crate::theme::{CELL_W, LINE_H};
use crate::App;

/// Which terminal holds the selection and the content generation it was made at.
#[derive(Default)]
pub struct Select {
    owner: Option<(u64, u64)>,
    /// The last link a ⌘-click opened (T2). Headless runs show it in the tree (`terminal/opened-url`), since they open nothing.
    pub opened: Option<String>,
    /// A drag held above or below the grid: it scrolls on every tick (T1b).
    auto: Option<Auto>,
    /// Whether ⌘ is down (T2b).
    pub cmd: bool,
    /// Where the pointer is over a pane's grid, in the grid's points (T2b).
    pub hover: Option<Hover>,
}

/// The pointer over pane `idx`'s grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hover { pub idx: usize, pub x: f32, pub y: f32 }

/// A link in the grid: where it points and the first and last cell it covers (rows the shell wrapped included).
#[derive(Clone, Debug, PartialEq)]
pub struct Link { pub url: String, pub from: Point, pub to: Point }

/// A selection drag whose pointer is outside the grid: the pane, which way to scroll (+1 back into the
/// scrollback, -1 towards the screen) and the pointer's last position in the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Auto { idx: usize, dir: i32, x: f32, y: f32 }

/// How often a held drag outside the grid scrolls one line.
pub const AUTO_EVERY: std::time::Duration = std::time::Duration::from_millis(40);

/// Which way a pointer at `y` scrolls a grid `h` high: +1 above it, -1 below it, 0 inside.
fn scroll_dir(y: f32, h: f32) -> i32 {
    if y < 0.0 { 1 } else if y >= h { -1 } else { 0 }
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
/// the word, a third the line), a drag selects from where it began.
pub fn drag(s: &mut App, idx: usize, d: &Drag, tx: &mut Tx) {
    let dir = if d.phase == Phase::Move && d.clicks == 1 { scroll_dir(d.y, d.h) } else { 0 };
    let was_idle = s.select.auto.is_none();
    s.select.auto = (dir != 0).then_some(Auto { idx, dir, x: d.x, y: d.y });
    if was_idle && dir != 0 { tx.wake_after(AUTO_EVERY); }
    let Some(t) = s.pane_tab(idx).and_then(|id| s.tabs.iter().find(|t| t.session.id() == id)) else { return };
    let id = t.session.id();
    let mut term = t.session.term().lock().unwrap();
    let generation = t.session.generation().load(Ordering::Acquire);
    match d.phase {
        Phase::Down => {
            term.selection = None;
            let kind = match d.clicks { 2 => Some(SelectionType::Semantic), 3 => Some(SelectionType::Lines), _ => None };
            if let Some(kind) = kind {
                let (p, side) = at(&term, d.x, d.y);
                term.selection = Some(Selection::new(kind, p, side));
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

/// One tick of a drag held outside the grid: scroll a line and stretch the selection to the grid's edge.
/// True while it goes on (the caller asks for the next tick).
pub fn autoscroll(s: &mut App) -> bool {
    let Some(a) = s.select.auto else { return false };
    let Some(t) = s.pane_tab(a.idx).and_then(|id| s.tabs.iter().find(|t| t.session.id() == id)) else {
        s.select.auto = None;
        return false;
    };
    t.session.scroll(a.dir);
    let mut term = t.session.term().lock().unwrap();
    let (p, side) = at(&term, a.x, a.y);
    if let Some(sel) = term.selection.as_mut() { sel.update(p, side); }
    true
}

/// The link under `(x, y)` of pane `idx`'s grid that ⌘-click opens (T2).
pub fn link_at(s: &App, idx: usize, x: f32, y: f32) -> Option<Link> {
    let id = s.pane_tab(idx)?;
    let t = s.tabs.iter().find(|t| t.session.id() == id)?;
    let term = t.session.term().lock().unwrap();
    link_in(&term, x, y)
}

/// The link the pointer is on while ⌘ is held, in pane `idx`'s terminal `term` (T2b).
pub fn hovered<T>(s: &App, idx: usize, term: &Term<T>) -> Option<Link> {
    let h = s.select.hover.filter(|h| s.select.cmd && h.idx == idx)?;
    link_in(term, h.x, h.y)
}

/// An OSC 8 hyperlink on the cell, else a URL written in the text. Only http, https and mailto count.
fn link_in<T>(term: &Term<T>, x: f32, y: f32) -> Option<Link> {
    let (p, _) = at(term, x, y);
    let link = match term.grid()[p].hyperlink().map(|h| h.uri().to_owned()) {
        Some(uri) => {
            // The run of cells on this row that carry the same target.
            let same = |c: usize| term.grid()[p.line][Column(c)].hyperlink().is_some_and(|h| h.uri() == uri);
            let mut a = p.column.0;
            while a > 0 && same(a - 1) { a -= 1; }
            let mut b = p.column.0;
            while b < term.last_column().0 && same(b + 1) { b += 1; }
            Link { url: uri.clone(), from: Point::new(p.line, Column(a)), to: Point::new(p.line, Column(b)) }
        }
        None => {
            let (chars, cells, idx) = logical_line(term, p);
            let span = url_span(&chars, idx)?;
            Link { url: chars[span.clone()].iter().collect(), from: cells[span.start], to: cells[span.end - 1] }
        }
    };
    openable(&link.url).then_some(link)
}

/// The text of the line `p` is on, rows joined where the shell wrapped them, the cell each character
/// came from, and the index of `p`'s character in it.
fn logical_line<T>(term: &Term<T>, p: Point) -> (Vec<char>, Vec<Point>, usize) {
    let last = term.last_column();
    let wrapped = |line: Line| term.grid()[line][last].flags.contains(Flags::WRAPLINE);
    let mut first = p.line;
    while first > term.topmost_line() && wrapped(first - 1) { first -= 1; }
    let mut chars = Vec::new();
    let mut cells = Vec::new();
    let mut at_p = 0;
    let mut line = first;
    loop {
        for c in 0..=last.0 {
            let cell = &term.grid()[line][Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                if line == p.line && c == p.column.0 { at_p = chars.len().saturating_sub(1); }
                continue;
            }
            if line == p.line && c == p.column.0 { at_p = chars.len(); }
            chars.push(if cell.c == '\0' { ' ' } else { cell.c });
            cells.push(Point::new(line, Column(c)));
        }
        if line >= term.bottommost_line() || !wrapped(line) { break; }
        line += 1;
    }
    (chars, cells, at_p)
}

/// Only these schemes open; anything else is left alone (Go D94).
fn openable(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    matches!(scheme.as_str(), "http" | "https" | "mailto")
}

/// Where the URL (`scheme://rest` or `mailto:rest`) in `chars` that covers index `at` is, without the
/// punctuation that ends a sentence (a closing bracket stays when its opening one is in the URL).
fn url_span(chars: &[char], at: usize) -> Option<std::ops::Range<usize>> {
    let is_scheme = |c: char| c.is_ascii_alphanumeric() || "+.-".contains(c);
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        i += 1;
        if !chars[start].is_ascii_alphabetic() || (start > 0 && is_scheme(chars[start - 1])) { continue; }
        let mut j = start;
        while j < chars.len() && is_scheme(chars[j]) { j += 1; }
        let scheme: String = chars[start..j].iter().collect::<String>().to_ascii_lowercase();
        let sep = if chars[j..].starts_with(&[':', '/', '/']) { 3 } else if scheme == "mailto" && chars.get(j) == Some(&':') { 1 } else { continue };
        let mut end = j + sep;
        while end < chars.len() && !chars[end].is_whitespace() && !"<>\"'`".contains(chars[end]) { end += 1; }
        while end > j + sep {
            let c = chars[end - 1];
            let open = match c { ')' => '(', ']' => '[', '}' => '{', _ => '\0' };
            let unbalanced = open != '\0' && chars[start..end].iter().filter(|&&x| x == open).count() < chars[start..end].iter().filter(|&&x| x == c).count();
            if ".,;:!?".contains(c) || unbalanced { end -= 1 } else { break }
        }
        if end > j + sep && (start..end).contains(&at) { return Some(start..end); }
        i = end.max(i);
    }
    None
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

    #[test]
    fn a_pointer_above_the_grid_scrolls_back_and_below_it_forward() {
        assert_eq!([scroll_dir(-0.5, 100.0), scroll_dir(0.0, 100.0), scroll_dir(99.9, 100.0), scroll_dir(100.0, 100.0)], [1, 0, 0, -1]);
    }

    fn url(text: &str, needle: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        let at = text.find(needle).unwrap();
        url_span(&chars, text[..at].chars().count()).map(|r| chars[r].iter().collect())
    }

    #[test]
    fn urls_are_found_under_the_pointer_without_sentence_punctuation() {
        let cases = [
            ("see https://example.com/docs. ok", "example", Some("https://example.com/docs")),
            ("see https://example.com/docs. ok", "see", None),
            ("(http://a.io/x_(y))", "a.io", Some("http://a.io/x_(y)")),
            ("(http://a.io/x)", "a.io", Some("http://a.io/x")),
            ("<https://a.io/p?q=1&r=2>, next", "q=1", Some("https://a.io/p?q=1&r=2")),
            ("go \"https://a.io\", now", "a.io", Some("https://a.io")),
            ("mail mailto:me@a.io; now", "me@", Some("mailto:me@a.io")),
            ("HTTPS://A.IO/x", "A.IO", Some("HTTPS://A.IO/x")),
            ("file:///etc/hosts here", "etc", Some("file:///etc/hosts")),
            ("a https://one.io b https://two.io c", "two", Some("https://two.io")),
            ("no scheme www.a.io here", "www", None),
            ("https:// alone", "https", None),
        ];
        for (text, needle, want) in cases {
            assert_eq!(url(text, needle).as_deref(), want, "{text:?} at {needle:?}");
        }
    }

    #[test]
    fn only_web_and_mail_schemes_open() {
        for (u, ok) in [("http://a.io", true), ("HTTPS://a.io", true), ("mailto:me@a.io", true), ("file:///etc/hosts", false), ("ftp://a.io", false), ("javascript:alert(1)", false)] {
            assert_eq!(openable(u), ok, "{u}");
        }
    }

    #[test]
    fn a_url_the_shell_wrapped_is_one_link_and_osc_8_links_carry_their_own_target() {
        let mut term = Term::new(Config::default(), &Dim, VoidListener);
        let mut p: Processor = Processor::new();
        // 20 columns: the URL runs from row 0 over row 1.
        p.advance(&mut term, b"go https://exampl.io/long/path ok\r\n\x1b]8;;https://target.io\x1b\\label\x1b]8;;\x1b\\");
        let (chars, cells, at_p) = logical_line(&term, Point::new(Line(1), Column(3)));
        let span = url_span(&chars, at_p).unwrap();
        assert_eq!(chars[span.clone()].iter().collect::<String>(), "https://exampl.io/long/path");
        // It starts on row 0 and ends on row 1.
        assert_eq!((cells[span.start].line.0, cells[span.end - 1].line.0), (0, 1));
        let cell = &term.grid()[Point::new(Line(2), Column(1))];
        assert_eq!(cell.hyperlink().map(|h| h.uri().to_owned()).as_deref(), Some("https://target.io"));
    }
}
