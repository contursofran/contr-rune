//! Search in the terminal (K7b): the Search button and ⌘F open the find bar, typing counts the
//! matches ("1 of 4"), Return / Shift+Return / ⌘G step with wrap, esc closes it, and keys typed
//! in the bar never reach the shell. The `find-text` scene prints four lines that hold "error" in
//! different cases. Elements: `find`, `find/input`, `find/count`, `find/prev`, `find/next`,
//! `find/close`, the title bar's `search`.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

/// Run the scene with `events` (the shell prints its lines by t = 4) and return the tree at t = 8.
fn run(scene: &str, mut events: Vec<Value>) -> Value {
    events.push(json!({ "t": 8.0, "type": "mark", "name": "end" }));
    let dir = std::env::temp_dir();
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let script = dir.join(format!("contr-rune-find-{id}.script.json"));
    let out = dir.join(format!("contr-rune-find-{id}.tree.json"));
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

fn bar_open(tree: &Value) -> bool {
    find(tree, "find").is_some()
}

fn count(tree: &Value) -> String {
    find(tree, "find/count").expect("the find bar is open")["text"].as_str().unwrap_or("").to_owned()
}

/// Every text on the page (grid rows included).
fn texts(tree: &Value) -> Vec<String> {
    tree["elements"].as_array().unwrap().iter().filter_map(|e| e["text"].as_str().map(str::to_owned)).collect()
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn typed(t: f64, text: &str) -> Value {
    json!({ "t": t, "type": "type", "text": text })
}

fn key(t: f64, name: &str) -> Value {
    json!({ "t": t, "type": "key", "name": name })
}

fn shift_key(t: f64, name: &str) -> Value {
    json!({ "t": t, "type": "key", "name": name, "shift": true })
}

/// ⌘F / ⌘G on macOS, Ctrl+Shift+F / G elsewhere; `back` is ⌘⇧G (Ctrl+Alt+Shift+G elsewhere).
fn chord(t: f64, name: &str, back: bool) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac || back, "alt": !mac && back })
}

const SCENE: &str = "find-text";

#[test]
fn the_bar_is_closed_until_asked_for() {
    let t = run(SCENE, vec![]);
    assert!(!bar_open(&t));
    assert!(find(&t, "search").is_some(), "the title bar has the Search button");
}

#[test]
fn the_search_button_opens_the_bar_at_the_top_right_of_the_terminal() {
    let t = run(SCENE, vec![tap(5.0, "search")]);
    assert!(bar_open(&t));
    for k in ["find/input", "find/count", "find/prev", "find/next", "find/close"] {
        assert!(find(&t, k).is_some(), "{k}");
    }
    let rect = |k: &str| find(&t, k).unwrap()["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    let (bar, card) = (rect("find"), rect("terminal"));
    assert_eq!(bar[1] - card[1], 10.0, "10 below the card's top");
    assert_eq!((card[0] + card[2]) - (bar[0] + bar[2]), 12.0, "12 from the card's right edge");
}

#[test]
fn the_chord_opens_it() {
    let t = run(SCENE, vec![chord(5.0, "f", false)]);
    assert!(bar_open(&t));
}

#[test]
fn typing_counts_the_matches_in_any_case() {
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "error")]);
    assert_eq!(count(&t), "1 of 4", "error, ERROR, Error and error");
    let none = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "zzq")]);
    assert_eq!(count(&none), "No results");
    let empty = run(SCENE, vec![tap(5.0, "search")]);
    assert_eq!(count(&empty), "", "no query, no count");
}

#[test]
fn the_query_is_plain_text() {
    // "." would match any character as a regex; here it matches only a dot (none on screen).
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "e.ror")]);
    assert_eq!(count(&t), "No results");
}

#[test]
fn next_and_previous_move_the_current_match_and_wrap() {
    let steps = |extra: Vec<Value>| {
        let mut ev = vec![tap(5.0, "search"), typed(5.5, "error")];
        ev.extend(extra);
        count(&run(SCENE, ev))
    };
    assert_eq!(steps(vec![key(6.0, "Enter")]), "2 of 4");
    assert_eq!(steps(vec![key(6.0, "Enter"), key(6.2, "Enter"), key(6.4, "Enter")]), "4 of 4");
    assert_eq!(steps((0..4).map(|i| key(6.0 + i as f64 * 0.2, "Enter")).collect()), "1 of 4", "next wraps to the first");
    assert_eq!(steps(vec![shift_key(6.0, "Enter")]), "4 of 4", "previous wraps to the last");
    assert_eq!(steps(vec![key(6.0, "Enter"), shift_key(6.2, "Enter")]), "1 of 4");
    assert_eq!(steps(vec![chord(6.0, "g", false)]), "2 of 4", "the chord steps forward");
    assert_eq!(steps(vec![chord(6.0, "g", true)]), "4 of 4", "the shifted chord steps back");
}

#[test]
fn the_next_and_previous_buttons_step_too() {
    let ev = vec![tap(5.0, "search"), typed(5.5, "error"), tap(6.0, "find/next"), tap(6.2, "find/next"), tap(6.4, "find/prev")];
    assert_eq!(count(&run(SCENE, ev)), "2 of 4");
}

#[test]
fn esc_and_the_close_button_close_the_bar() {
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "error"), key(6.0, "Escape")]);
    assert!(!bar_open(&t), "esc closes it");
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "error"), tap(6.0, "find/close")]);
    assert!(!bar_open(&t), "the close button closes it");
}

#[test]
fn the_bar_reopens_and_keeps_working_after_esc() {
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "error"), key(6.0, "Escape"), tap(6.5, "search"), key(7.0, "Enter")]);
    assert!(bar_open(&t));
    assert_eq!(count(&t), "2 of 4", "the old query is back (1 of 4) and Return steps to the second");
}

#[test]
fn keys_typed_in_the_bar_do_not_reach_the_shell() {
    // Had "zzqword" gone to the shell, its echo would show in the grid.
    let t = run(SCENE, vec![tap(5.0, "search"), typed(5.5, "zzqword"), key(6.0, "Enter")]);
    let grid_rows = |t: &Value| -> Vec<String> {
        t["elements"].as_array().unwrap().iter()
            .filter(|e| e["key"].as_str().is_some_and(|k| k.starts_with('l') && k[1..].parse::<u32>().is_ok()))
            .filter_map(|e| e["text"].as_str().map(str::to_owned))
            .collect()
    };
    assert!(!grid_rows(&t).iter().any(|s| s.contains("zzqword")), "no row of the grid shows it");
    assert_eq!(count(&t), "No results");
    // After esc, typing reaches the shell again.
    let t = run(SCENE, vec![tap(5.0, "search"), key(5.5, "Escape"), typed(6.0, "uname"), key(6.5, "Enter")]);
    assert!(texts(&t).iter().any(|s| matches!(s.trim(), "Darwin" | "Linux")), "the shell runs uname after the bar is closed");
}

#[test]
fn another_tab_starts_without_the_bar() {
    let t = run("three-tabs", vec![tap(5.0, "search"), typed(5.5, "a"), tap(6.0, "new-tab")]);
    assert!(!bar_open(&t), "a new tab closes the bar and its highlights");
}
