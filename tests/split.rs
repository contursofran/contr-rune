//! Split terminal (K7c): the Split button and ⌘D open a second terminal on the right, focus moves
//! by click and ⌘[ / ⌘], typing goes only to the focused pane, the divider resizes (each pane keeps
//! a fifth), and the panes un-split by the button, ⌘W and a shell that exits. Elements: the title
//! bar's `split`; `panes` (the row), `terminal/body` / `terminal/body/1` (the panes), `grid` /
//! `grid/1` (their grids), rows `l<N>` / `p1/l<N>`, `split/divider`; tabs `t-<id>`, sidebar rows
//! `term-<id>`.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

/// Run `scene` with `events` and return the tree at t = 10 (the shells are up by t = 4).
fn run(scene: &str, events: Vec<Value>) -> Value { run_until(scene, events, 10.0) }

/// Like `run`, with the tree taken at t = `end`. Script time is not wall time: a frame costs about a
/// millisecond, so a run lasts about `end / 16` seconds, and the real shells (their start-up and the
/// scene's typed text) only get that long. A later `end` gives them more time on a loaded machine.
fn run_until(scene: &str, mut events: Vec<Value>, end: f64) -> Value {
    events.push(json!({ "t": end, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir();
    let script = dir.join(format!("contr-rune-split-{id}.script.json"));
    let out = dir.join(format!("contr-rune-split-{id}.tree.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .args([
            "--script", script.to_str().unwrap(),
            "--tree", out.to_str().unwrap(),
            "--size", "1710x1112", "--scale", "1", "--scene", scene, "--at", "mark:end",
        ])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_file(script);
    let _ = std::fs::remove_file(out);
    v
}

fn find<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key)
}

fn rect(tree: &Value, key: &str) -> Vec<f64> {
    let e = find(tree, key).unwrap_or_else(|| panic!("no element {key}"));
    e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

/// The rows of one pane's grid as text: `prefix` is "l" for the left pane, "p1/l" for the right one.
fn rows(tree: &Value, prefix: &str) -> String {
    tree["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["key"].as_str().is_some_and(|k| k.strip_prefix(prefix).is_some_and(|n| n.parse::<u32>().is_ok())))
        .filter_map(|e| e["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn left(tree: &Value) -> String { rows(tree, "l") }
fn right(tree: &Value) -> String { rows(tree, "p1/l") }

fn split_open(tree: &Value) -> bool { find(tree, "grid/1").is_some() }

fn tap(t: f64, key: &str) -> Value { json!({ "t": t, "type": "tap", "key": key }) }
fn typed(t: f64, text: &str) -> Value { json!({ "t": t, "type": "type", "text": text }) }
fn enter(t: f64) -> Value { json!({ "t": t, "type": "key", "name": "Enter" }) }
fn drag(t: f64, dx: f64) -> Value { json!({ "t": t, "type": "drag", "key": "split/divider", "dx": dx, "dur": 0.3 }) }

/// ⌘ + `name` on macOS, Ctrl+Shift + `name` elsewhere.
fn chord(t: f64, name: &str) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac })
}

fn selected_tab(tree: &Value) -> String {
    let found: Vec<&Value> = tree["elements"].as_array().unwrap().iter().filter(|e| e["role"] == "tab" && e["itemSelected"] == true).collect();
    assert_eq!(found.len(), 1, "one selected tab, got {found:?}");
    found[0]["key"].as_str().unwrap().trim_start_matches("t-").to_owned()
}

fn row_selected(tree: &Value, key: &str) -> bool {
    find(tree, key).unwrap_or_else(|| panic!("no {key}"))["itemSelected"] == true
}

/// The PTY's columns from a `stty size` line ("<rows> <cols>") in `text`.
fn pty_cols(text: &str) -> f64 {
    text.lines()
        .rev()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            match p.as_slice() {
                [r, c] if r.parse::<u32>().is_ok() => c.parse::<u32>().ok(),
                _ => None,
            }
        })
        .next()
        .map(f64::from)
        .unwrap_or_else(|| panic!("no `stty size` output in {text:?}"))
}

const SCENE: &str = "find-text";
const CELL_W: f64 = 13.0 * 0.6;

#[test]
fn one_panel_until_split() {
    let t = run(SCENE, vec![]);
    assert!(!split_open(&t));
    assert!(find(&t, "split/divider").is_none());
}

#[test]
fn the_button_splits_into_two_halves() {
    let off = run(SCENE, vec![]);
    let t = run(SCENE, vec![tap(5.0, "split")]);
    assert!(split_open(&t), "a second grid");
    let (card, a, b) = (rect(&t, "terminal"), rect(&t, "grid"), rect(&t, "grid/1"));
    assert!((a[2] - b[2]).abs() <= 2.0, "halves: {} and {}", a[2], b[2]);
    // Half the card, less the 20 point padding each side and the pane's 1 point border.
    assert!((a[2] - (card[2] / 2.0 - 42.0)).abs() <= 3.0, "about half: {} of a {} card", a[2], card[2]);
    assert!(b[0] > a[0] + a[2], "the right grid is to the right");
    // A new terminal in the same project: a tab and a sidebar row, selected, in the right pane.
    assert_eq!(selected_tab(&off), "1");
    assert_eq!(selected_tab(&t), "2");
    assert!(row_selected(&t, "term-2") && !row_selected(&t, "term-1"));
    assert!(find(&t, "t-1").is_some() && find(&t, "term-1").is_some());
    // The left pane keeps the first terminal's text.
    assert!(left(&t).contains("all done"), "{}", left(&t));
    assert!(!right(&t).contains("all done"));
    assert_ne!(find(&t, "split").unwrap()["fill"], find(&off, "split").unwrap()["fill"], "the button looks pressed while split");
}

#[test]
fn the_chord_splits_and_unsplits() {
    let t = run(SCENE, vec![chord(5.0, "d")]);
    assert!(split_open(&t));
    let t = run(SCENE, vec![chord(5.0, "d"), chord(6.5, "d")]);
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "2", "the focused (right) terminal takes the panel");
    assert!(find(&t, "t-1").is_some(), "the other terminal stays open as a tab");
}

#[test]
fn the_button_unsplits_and_the_focused_pane_takes_the_panel() {
    // Focus the left pane, then un-split: terminal 1 (with the find text) is the whole panel.
    let t = run(SCENE, vec![tap(5.0, "split"), tap(6.0, "grid"), tap(7.0, "split")]);
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "1");
    assert!(left(&t).contains("all done"));
    let (card, g) = (rect(&t, "terminal"), rect(&t, "grid"));
    assert_eq!(g[2], card[2] - 40.0, "the card less the 20 point padding each side");
}

#[test]
fn each_pty_has_its_own_width() {
    let t = run(SCENE, vec![
        tap(5.0, "split"),
        typed(6.0, "stty size"), enter(6.8),
        tap(7.5, "grid"),
        typed(8.0, "stty size"), enter(8.8),
    ]);
    let (a, b) = (rect(&t, "grid"), rect(&t, "grid/1"));
    assert_eq!(pty_cols(&left(&t)), (a[2] / CELL_W).floor(), "left cols for a {} wide grid", a[2]);
    assert_eq!(pty_cols(&right(&t)), (b[2] / CELL_W).floor(), "right cols for a {} wide grid", b[2]);
    // Un-split: the PTY grows to the full width.
    let t = run(SCENE, vec![tap(5.0, "split"), tap(6.0, "split"), typed(7.0, "stty size"), enter(7.8)]);
    let g = rect(&t, "grid");
    assert_eq!(pty_cols(&left(&t)), (g[2] / CELL_W).floor());
    assert!(g[2] > a[2] * 1.9);
}

#[test]
fn focus_follows_clicks_and_the_bracket_keys() {
    let t = run(SCENE, vec![tap(5.0, "split")]);
    assert_eq!(selected_tab(&t), "2");
    let t = run(SCENE, vec![tap(5.0, "split"), tap(6.0, "grid")]);
    assert_eq!(selected_tab(&t), "1", "a click on the left pane focuses it; the strip follows");
    assert!(row_selected(&t, "term-1") && !row_selected(&t, "term-2"), "so does the sidebar");
    let t = run(SCENE, vec![tap(5.0, "split"), tap(6.0, "grid"), tap(7.0, "grid/1")]);
    assert_eq!(selected_tab(&t), "2");
    // ⌘[ moves left, ⌘] right.
    let t = run(SCENE, vec![tap(5.0, "split"), chord(6.0, "[")]);
    assert_eq!(selected_tab(&t), "1");
    let t = run(SCENE, vec![tap(5.0, "split"), chord(6.0, "["), chord(7.0, "]")]);
    assert_eq!(selected_tab(&t), "2");
    assert!(split_open(&t), "moving the focus keeps the split");
}

#[test]
fn typing_goes_only_to_the_focused_pane() {
    let t = run(SCENE, vec![
        tap(5.0, "split"),
        typed(6.0, "printf '%s%s\\n' MARK ONE"), enter(7.0),
        tap(7.5, "grid"),
        typed(8.0, "printf '%s%s\\n' MARK TWO"), enter(9.0),
    ]);
    let (l, r) = (left(&t), right(&t));
    assert!(r.contains("MARKONE") && !r.contains("MARKTWO"), "right: {r}");
    assert!(l.contains("MARKTWO") && !l.contains("MARKONE"), "left: {l}");
}

#[test]
fn selecting_a_tab_shows_it_in_the_focused_pane() {
    // Split from tab 3: left shows 3, right the new tab 4.
    let t = run("three-tabs", vec![tap(5.0, "split")]);
    assert_eq!(selected_tab(&t), "4");
    let t = run("three-tabs", vec![tap(5.0, "split"), tap(6.0, "t-1")]);
    assert!(split_open(&t));
    assert_eq!(selected_tab(&t), "1", "tab 1 now fills the focused (right) pane");
    assert!(row_selected(&t, "term-1") && !row_selected(&t, "term-4"));
    // Selecting the tab of the other pane moves the focus there instead.
    let t = run("three-tabs", vec![tap(5.0, "split"), tap(6.0, "t-3")]);
    assert_eq!(selected_tab(&t), "3");
    assert!(split_open(&t));
    // A sidebar row does the same as its tab.
    let t = run("three-tabs", vec![tap(5.0, "split"), tap(6.0, "term-3")]);
    assert_eq!(selected_tab(&t), "3");
}

#[test]
fn dragging_the_divider_resizes_and_keeps_a_fifth() {
    let t = run(SCENE, vec![tap(5.0, "split")]);
    let (row0, a0) = (rect(&t, "panes"), rect(&t, "grid"));
    let t = run(SCENE, vec![tap(5.0, "split"), drag(6.0, 200.0)]);
    let a1 = rect(&t, "grid");
    assert!((a1[2] - a0[2] - 200.0).abs() <= 3.0, "the left grid grew by the drag: {} to {}", a0[2], a1[2]);
    assert!(rect(&t, "grid/1")[2] < a0[2] - 190.0, "the right one shrank");
    // A drag far past the edge stops at a fifth of the panel for the left pane...
    let t = run(SCENE, vec![tap(5.0, "split"), drag(6.0, -2000.0)]);
    let a = rect(&t, "terminal/body");
    assert!((a[2] - row0[2] * 0.2).abs() <= 2.0, "left pane is a fifth: {} of {}", a[2], row0[2]);
    // ...and for the right pane.
    let t = run(SCENE, vec![tap(5.0, "split"), drag(6.0, 2000.0)]);
    let b = rect(&t, "terminal/body/1");
    assert!((b[2] - row0[2] * 0.2).abs() <= 2.0, "right pane is a fifth: {} of {}", b[2], row0[2]);
}

#[test]
fn closing_a_terminal_unsplits() {
    // Terminal 1's shell must run the scene's text ("all done") before the end, even on a loaded machine.
    const END: f64 = 40.0;
    // ⌘W closes the focused (right) terminal.
    let t = run_until(SCENE, vec![tap(5.0, "split"), chord(6.0, "w")], END);
    assert!(!split_open(&t));
    assert!(find(&t, "t-2").is_none() && find(&t, "t-1").is_some());
    assert_eq!(selected_tab(&t), "1");
    // The shell exiting in the right pane does the same.
    let t = run_until(SCENE, vec![tap(5.0, "split"), typed(6.0, "exit"), enter(6.8)], END);
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "1");
    assert!(left(&t).contains("all done"));
    // A shell exiting in the focused left pane leaves the right terminal as the whole panel.
    let t = run_until(SCENE, vec![tap(5.0, "split"), tap(6.0, "grid"), typed(6.5, "exit"), enter(7.3)], END);
    assert!(!split_open(&t));
    assert_eq!(selected_tab(&t), "2");
}
