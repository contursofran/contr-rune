//! The terminal panel (K3): the grid sits in the card with the target padding (14 top, 20 sides,
//! 12 bottom), there is no header row, and typed text reaches the shell and shows in the grid.
//! Elements: `terminal` (card), `terminal/body` (padded area), `grid`, rows `l<N>`.

use std::process::Command;

use serde_json::{Value, json};

fn run(name: &str, mut events: Vec<Value>) -> Value {
    // Read the tree at a mark, late enough for the shells to start and for the PTY to resize.
    events.push(json!({ "t": 8.0, "type": "mark", "name": "end" }));
    let dir = std::env::temp_dir();
    let script = dir.join(format!("contr-rune-{name}-{}.script.json", std::process::id()));
    let out = dir.join(format!("contr-rune-{name}-{}.tree.json", std::process::id()));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .args([
            "--script", script.to_str().unwrap(),
            "--tree", out.to_str().unwrap(),
            "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs", "--at", "mark:end",
        ])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_file(script);
    let _ = std::fs::remove_file(out);
    v
}

fn find<'a>(tree: &'a Value, key: &str) -> &'a Value {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key).unwrap_or_else(|| panic!("no element {key}"))
}

fn rect(tree: &Value, key: &str) -> Vec<f64> {
    find(tree, key)["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

#[test]
fn grid_sits_in_the_card_with_the_target_padding() {
    let t = run("bounds", vec![]);
    let card = rect(&t, "terminal");
    let grid = rect(&t, "grid");
    // 14 top, 20 left/right, 12 bottom.
    assert_eq!(grid[0] - card[0], 20.0, "left padding");
    assert_eq!(grid[1] - card[1], 14.0, "top padding");
    assert_eq!((card[0] + card[2]) - (grid[0] + grid[2]), 20.0, "right padding");
    assert_eq!((card[1] + card[3]) - (grid[1] + grid[3]), 12.0, "bottom padding");
    // The first row starts at the grid's top: nothing (no header row) sits above it, and rows are 20 high.
    assert_eq!(rect(&t, "l0")[1], grid[1]);
    assert_eq!(rect(&t, "l0")[3], 20.0);
    assert_eq!(rect(&t, "l1")[1], grid[1] + 20.0);
}

#[test]
fn rows_fill_the_grid_height() {
    let t = run("rows", vec![]);
    let grid = rect(&t, "grid");
    let rows = t["elements"].as_array().unwrap().iter().filter(|e| e["role"] == "text" && e["key"].as_str().is_some_and(|k| k.starts_with('l') && k[1..].parse::<u32>().is_ok())).count();
    assert_eq!(rows as f64, (grid[3] / 20.0).floor(), "one text row per 20 px of grid height");
}

#[test]
fn typed_text_reaches_the_shell_and_shows_in_the_grid() {
    // `uname` is one word (the `type` event does not keep spaces in order with the next key); its output
    // is a line of its own and cannot be the echoed command.
    let t = run(
        "type",
        vec![
            json!({ "t": 4.0, "type": "type", "text": "uname" }),
            json!({ "t": 4.2, "type": "key", "name": "Enter" }),
        ],
    );
    let shown = t["elements"].as_array().unwrap().iter().any(|e| e["text"].as_str().is_some_and(|s| matches!(s.trim(), "Darwin" | "Linux")));
    assert!(shown, "the grid shows the shell's output");
}
