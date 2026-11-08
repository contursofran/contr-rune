//! Search in the terminal (K7b): the helpers that turn a typed word into matches over the
//! scrollback and screen, and the state of the find bar.
//!
//! Matching is plain text, case-insensitive: the query is escaped and run through
//! alacritty_terminal's regex search. The matches are recomputed from the emulator whenever the
//! view or a step needs them, so new output keeps the count live.

use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use rune::TextEdit;

pub type Match = RangeInclusive<Point>;

/// The find bar's state. The matches are not kept here: they come from the emulator each time.
pub struct Find {
    pub open: bool,
    pub edit: TextEdit,
    /// The current match, an index into the matches (clamped when the count shrinks).
    pub current: usize,
    cache: Mutex<Option<Cached>>,
    /// How many times the regex search ran (tests read it to see that matches are reused).
    runs: AtomicUsize,
}

/// The matches of one search and what they were found for.
struct Cached {
    terminal: u64,
    generation: u64,
    query: String,
    matches: Arc<Vec<Match>>,
}

impl Find {
    pub fn new() -> Find {
        Find { open: false, edit: TextEdit::new(""), current: 0, cache: Mutex::new(None), runs: AtomicUsize::new(0) }
    }

    /// The matches of the query in terminal `terminal` (whose content is at `generation`): the last
    /// search's when the query, the terminal and its generation are the same, else a new search.
    pub fn matches<T: EventListener>(&self, terminal: u64, generation: u64, term: &Term<T>) -> Arc<Vec<Match>> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(c) = cache.as_ref()
            && c.terminal == terminal && c.generation == generation && c.query == self.query()
        {
            return c.matches.clone();
        }
        self.runs.fetch_add(1, Ordering::Relaxed);
        let matches = Arc::new(matches(term, self.query()));
        *cache = Some(Cached { terminal, generation, query: self.query().to_owned(), matches: matches.clone() });
        matches
    }

    /// How many searches ran so far.
    #[cfg(test)]
    pub fn runs(&self) -> usize {
        self.runs.load(Ordering::Relaxed)
    }

    /// Close the bar; its highlights go with it. The query stays for the next ⌘F.
    pub fn close(&mut self) {
        self.open = false;
        self.edit.focused = false;
        self.current = 0;
        *self.cache.lock().unwrap() = None;
    }

    pub fn query(&self) -> &str {
        &self.edit.text
    }
}

/// `query` as a regex that matches it literally, in any case.
pub fn pattern(query: &str) -> String {
    let mut out = String::from("(?i)");
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Every match of `query` in the scrollback and screen, from the top down. Empty for an empty query.
pub fn matches<T: EventListener>(term: &Term<T>, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(mut regex) = RegexSearch::new(&pattern(query)) else { return Vec::new() };
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    RegexIter::new(start, end, Direction::Right, term, &mut regex).collect()
}

/// The match after (`forward`) or before `current`, wrapping at both ends. `None` when there are no matches.
pub fn step(current: usize, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = current.min(len - 1);
    Some(if forward { (current + 1) % len } else { (current + len - 1) % len })
}

/// The first match at or below the top of the view (`display_offset` lines scrolled up), else the last one.
pub fn first_in_view(matches: &[Match], display_offset: usize) -> usize {
    let top = -(display_offset as i32);
    matches.iter().position(|m| m.start().line.0 >= top).unwrap_or(matches.len().saturating_sub(1))
}

/// How far to scroll the view (`Scroll::Delta`, positive is up into the history) so that `m` shows:
/// 0 when it is already visible, else the match is centred.
pub fn scroll_delta<T>(term: &Term<T>, m: &Match) -> i32 {
    let (rows, offset) = (term.screen_lines() as i32, term.grid().display_offset() as i32);
    let line = m.start().line.0;
    if line >= -offset && line < rows - offset {
        return 0;
    }
    let wanted = (rows / 2 - line).clamp(0, term.history_size() as i32);
    wanted - offset
}

/// Which match, if any, covers `p`. `matches` are sorted and do not overlap.
pub fn at(matches: &[Match], p: Point) -> Option<usize> {
    let i = matches.partition_point(|m| *m.start() <= p).checked_sub(1)?;
    (*matches[i].end() >= p).then_some(i)
}

/// The count shown in the bar: "3 of 12", "No results", or nothing for an empty query.
pub fn count_label(query: &str, current: usize, total: usize) -> String {
    match (query.is_empty(), total) {
        (true, _) => String::new(),
        (false, 0) => "No results".into(),
        _ => format!("{} of {total}", current.min(total - 1) + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::Line;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    struct Quiet;
    impl EventListener for Quiet {}

    struct Dim(usize, usize);
    impl Dimensions for Dim {
        fn total_lines(&self) -> usize { self.0 }
        fn screen_lines(&self) -> usize { self.0 }
        fn columns(&self) -> usize { self.1 }
    }

    fn term(rows: usize, cols: usize, text: &str) -> Term<Quiet> {
        let mut t = Term::new(Config { scrolling_history: 100, ..Config::default() }, &Dim(rows, cols), Quiet);
        let mut p: Processor = Processor::new();
        p.advance(&mut t, text.as_bytes());
        t
    }

    fn found(t: &Term<Quiet>, q: &str) -> Vec<(i32, usize, usize)> {
        matches(t, q).iter().map(|m| (m.start().line.0, m.start().column.0, m.end().column.0)).collect()
    }

    #[test]
    fn the_query_is_plain_text() {
        let t = term(5, 40, "a.c abc a+c\r\n(x) [y]\r\n");
        assert_eq!(found(&t, "a.c"), vec![(0, 0, 2)], "the dot is not a wildcard");
        assert_eq!(found(&t, "a+c"), vec![(0, 8, 10)]);
        assert_eq!(found(&t, "(x)"), vec![(1, 0, 2)]);
        assert_eq!(found(&t, "[y]"), vec![(1, 4, 6)]);
        assert_eq!(found(&t, "\\"), vec![]);
        assert_eq!(pattern("a.b"), "(?i)a\\.b");
    }

    #[test]
    fn the_search_ignores_case_even_with_capitals() {
        let t = term(5, 40, "Foo foo FOO\r\n");
        assert_eq!(found(&t, "foo").len(), 3);
        assert_eq!(found(&t, "FoO").len(), 3);
    }

    #[test]
    fn an_empty_query_has_no_matches() {
        assert!(matches(&term(3, 10, "abc\r\n"), "").is_empty());
    }

    #[test]
    fn the_scrollback_is_searched_too() {
        let mut text = String::from("needle\r\n");
        for i in 0..10 { text += &format!("line {i}\r\n"); }
        let t = term(4, 20, &text);
        let m = found(&t, "needle");
        assert_eq!(m.len(), 1);
        assert!(m[0].0 < 0, "the match is above the screen, in history: {m:?}");
    }

    #[test]
    fn a_match_can_wrap_across_rows() {
        let t = term(3, 5, "abcdefgh");
        assert_eq!(matches(&t, "defg").len(), 1);
    }

    #[test]
    fn stepping_wraps_both_ways() {
        assert_eq!(step(0, 3, true), Some(1));
        assert_eq!(step(2, 3, true), Some(0));
        assert_eq!(step(0, 3, false), Some(2));
        assert_eq!(step(1, 3, false), Some(0));
        assert_eq!(step(5, 3, true), Some(0), "a stale index is clamped first");
        assert_eq!(step(0, 0, true), None);
    }

    #[test]
    fn a_hit_test_finds_the_match_under_a_point() {
        let t = term(3, 20, "ab ab\r\n");
        let m = matches(&t, "ab");
        let p = |c| Point::new(Line(0), Column(c));
        assert_eq!(at(&m, p(0)), Some(0));
        assert_eq!(at(&m, p(1)), Some(0));
        assert_eq!(at(&m, p(2)), None);
        assert_eq!(at(&m, p(4)), Some(1));
        assert_eq!(at(&m, p(9)), None);
        assert_eq!(at(&[], p(0)), None);
    }

    #[test]
    fn scrolling_shows_a_hidden_match_and_leaves_a_visible_one() {
        let mut text = String::from("needle\r\n");
        for i in 0..30 { text += &format!("line {i}\r\n"); }
        let t = term(10, 20, &text);
        let m = matches(&t, "needle");
        let d = scroll_delta(&t, &m[0]);
        assert!(d > 0, "scroll up into the history");
        let line = m[0].start().line.0;
        assert!(line + d >= 0 && line + d < 10, "the match lands on screen");
        let last = matches(&t, "line 29");
        assert_eq!(scroll_delta(&t, &last[0]), 0);
    }

    #[test]
    fn matches_are_reused_until_the_query_the_terminal_or_the_output_changes() {
        let mut f = Find::new();
        f.edit.replace_text("foo");
        let mut t = term(5, 40, "foo foo\r\n");
        assert_eq!(f.matches(1, 0, &t).len(), 2);
        assert_eq!(f.runs(), 1);
        // Nothing changed (a redraw): no new search, the same matches.
        let again = f.matches(1, 0, &t);
        assert_eq!((again.len(), f.runs()), (2, 1));
        // New output: the generation moves, the count follows.
        let mut p: Processor = Processor::new();
        p.advance(&mut t, b"foo\r\n");
        assert_eq!(f.matches(1, 1, &t).len(), 3);
        assert_eq!(f.runs(), 2);
        // Another terminal, then another query.
        f.matches(2, 1, &t);
        assert_eq!(f.runs(), 3);
        f.edit.replace_text("fo");
        f.matches(2, 1, &t);
        assert_eq!(f.runs(), 4);
        f.matches(2, 1, &t);
        assert_eq!(f.runs(), 4);
        // Closing drops the cache.
        f.close();
        f.matches(2, 1, &t);
        assert_eq!(f.runs(), 5);
    }

    #[test]
    fn the_count_label_reads_like_the_bar() {
        assert_eq!(count_label("", 0, 0), "");
        assert_eq!(count_label("x", 0, 0), "No results");
        assert_eq!(count_label("x", 2, 12), "3 of 12");
        assert_eq!(count_label("x", 9, 4), "4 of 4");
    }
}
