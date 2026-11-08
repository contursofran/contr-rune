//! The title bar's tab strip (K2): `--script` drives clicks and keys against the `three-tabs`
//! scene (three real shells) and `--tree` reads the result. Tab elements are `t-<id>`, the
//! close button is `t-<id>/.../close`, the `+` button is `new-tab`.

use std::process::Command;

use serde_json::{Value, json};

fn run(name: &str, events: Vec<Value>) -> Value {
    let dir = std::env::temp_dir();
    let script = dir.join(format!("contr-rune-{name}-{}.script.json", std::process::id()));
    let out = dir.join(format!("contr-rune-{name}-{}.tree.json", std::process::id()));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .args([
            "--script", script.to_str().unwrap(),
            "--tree", out.to_str().unwrap(),
            "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs",
        ])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_file(script);
    let _ = std::fs::remove_file(out);
    v
}

fn elements(tree: &Value) -> &Vec<Value> {
    tree["elements"].as_array().unwrap()
}

/// The tab ids in strip order (left to right).
fn tab_ids(tree: &Value) -> Vec<String> {
    let mut tabs: Vec<(f64, String)> = elements(tree)
        .iter()
        .filter(|e| e["role"] == "tab" && e["key"].as_str().is_some_and(|k| k.starts_with("t-")))
        .map(|e| (e["rect"][0].as_f64().unwrap(), e["key"].as_str().unwrap().trim_start_matches("t-").to_owned()))
        .collect();
    tabs.sort_by(|a, b| a.0.total_cmp(&b.0));
    tabs.into_iter().map(|t| t.1).collect()
}

/// The id of the selected tab: the sidebar's "Terminals" rows are not tabs, so use the strip's
/// own `itemSelected` flag on the `tab` role.
fn active_id(tree: &Value) -> String {
    let found: Vec<&Value> = elements(tree)
        .iter()
        .filter(|e| e["role"] == "tab" && e["itemSelected"] == true)
        .collect();
    assert_eq!(found.len(), 1, "one selected tab, got {found:?}");
    found[0]["key"].as_str().unwrap().trim_start_matches("t-").to_owned()
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn key(t: f64, name: &str, ctrl: bool, shift: bool) -> Value {
    json!({ "t": t, "type": "key", "name": name, "ctrl": ctrl, "shift": shift })
}

/// The window chord for a letter: ⌘ on macOS, Ctrl+Shift elsewhere.
fn chord(t: f64, letter: &str) -> Value {
    if cfg!(target_os = "macos") {
        json!({ "t": t, "type": "key", "name": letter, "cmd": true })
    } else {
        json!({ "t": t, "type": "key", "name": letter, "ctrl": true, "shift": true })
    }
}

#[test]
fn starts_with_three_tabs_and_the_last_selected() {
    let t = run("start", vec![]);
    assert_eq!(tab_ids(&t), ["1", "2", "3"]);
    assert_eq!(active_id(&t), "3");
    // The same bounds as `kit_preview --tree`: 180 x 28 tabs 6 apart from x 342, then `+`.
    let rect = |key: &str| -> Vec<f64> {
        let e = elements(&t).iter().find(|e| e["key"] == key).unwrap_or_else(|| panic!("no {key}"));
        e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    };
    assert_eq!(rect("t-1"), [342.0, 5.0, 180.0, 28.0]);
    assert_eq!(rect("t-2"), [528.0, 5.0, 180.0, 28.0]);
    assert_eq!(rect("t-3"), [714.0, 5.0, 180.0, 28.0]);
    assert_eq!(rect("top-list"), [342.0, 5.0, 552.0, 28.0]);
}

#[test]
fn click_selects_a_tab() {
    let t = run("click", vec![tap(0.5, "t-1")]);
    assert_eq!(active_id(&t), "1");
    assert_eq!(tab_ids(&t).len(), 3);
}

#[test]
fn close_removes_the_tab() {
    // The close button sits under the tab; the strip keys it `close`.
    let before = run("close0", vec![]);
    let close = elements(&before)
        .iter()
        .find(|e| e["key"] == "close" && e["path"].as_str().unwrap().contains("/t-2/"))
        .expect("close button on t-2");
    let r = &close["rect"];
    let (x, y) = (r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0, r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0);
    // Hover first: the close button only takes the click once the tab shows it.
    let t = run(
        "close",
        vec![
            json!({ "t": 0.3, "type": "move", "key": "t-2" }),
            json!({ "t": 0.6, "type": "tap", "x": x, "y": y }),
        ],
    );
    assert_eq!(tab_ids(&t), ["1", "3"]);
}

#[test]
fn plus_adds_a_tab_and_selects_it() {
    let t = run("plus", vec![tap(0.5, "new-tab")]);
    assert_eq!(tab_ids(&t), ["1", "2", "3", "4"]);
    assert_eq!(active_id(&t), "4");
}

#[test]
fn chords_open_and_close_tabs() {
    let t = run("chords", vec![chord(0.5, "t")]);
    assert_eq!(tab_ids(&t), ["1", "2", "3", "4"]);
    assert_eq!(active_id(&t), "4");
    let t = run("chords-w", vec![chord(0.5, "w")]);
    assert_eq!(tab_ids(&t), ["1", "2"]);
    assert_eq!(active_id(&t), "2");
}

#[test]
fn ctrl_tab_cycles_both_ways() {
    let t = run("ctrl-tab", vec![key(0.5, "Tab", true, false)]);
    assert_eq!(active_id(&t), "1", "forward from the last tab wraps to the first");
    let t = run("ctrl-shift-tab", vec![key(0.5, "Tab", true, true)]);
    assert_eq!(active_id(&t), "2");
    let t = run("ctrl-tab-2", vec![key(0.5, "Tab", true, false), key(0.7, "Tab", true, false)]);
    assert_eq!(active_id(&t), "2");
}

#[test]
fn ctrl_page_keys_cycle() {
    let t = run("ctrl-pgdn", vec![key(0.5, "PageDown", true, false)]);
    assert_eq!(active_id(&t), "1");
    let t = run("ctrl-pgup", vec![key(0.5, "PageUp", true, false)]);
    assert_eq!(active_id(&t), "2");
}

#[test]
fn dragging_a_tab_reorders_the_strip() {
    let t = run(
        "reorder",
        vec![json!({ "t": 0.5, "type": "drag", "key": "t-1", "dx": 200.0, "dy": 0.0, "dur": 0.6 })],
    );
    assert_eq!(tab_ids(&t), ["2", "1", "3"]);
}

#[test]
fn no_project_name_on_the_title_bar_right() {
    // The diff summary (K5) is the only text there; it is not a project name.
    let t = run("right", vec![]);
    let bar = elements(&t).iter().find(|e| e["key"] == "titlebar").unwrap();
    let bar_right = bar["rect"][0].as_f64().unwrap() + bar["rect"][2].as_f64().unwrap();
    let texts_right: Vec<&Value> = elements(&t)
        .iter()
        .filter(|e| e["path"].as_str().is_some_and(|p| p.contains("titlebar/") && !p.contains("diff-summary")) && e["rect"][0].as_f64().unwrap() > 1000.0 && e["text"].is_string())
        .collect();
    assert!(texts_right.is_empty(), "nothing on the right of the title bar before {bar_right}: {texts_right:?}");
}
