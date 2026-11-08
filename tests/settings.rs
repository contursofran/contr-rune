//! The settings dialog and the light theme (K7a): the gear and ⌘, open the dialog; esc, Done and
//! the scrim close it; choosing Light repaints the app with `Kit::neutral(false)` and the choice
//! lives in the settings file (`CONTR_SETTINGS` points it at a temp file here).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rune_kit::Kit;
use serde_json::{Value, json};

fn temp(name: &str) -> PathBuf {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("contr-rune-set-{name}-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run the app headless with `events` and the settings file at `settings`; return the tree.
fn run(settings: &Path, events: Vec<Value>) -> Value {
    let dir = temp("run");
    let (script, out) = (dir.join("script.json"), dir.join("tree.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", settings)
        .env("CONTR_PROJECTS", std::env::temp_dir())
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs"])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    v
}

fn find<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key)
}

fn open(tree: &Value) -> bool {
    find(tree, "settings-dialog").is_some()
}

fn fill(tree: &Value, key: &str) -> String {
    find(tree, key).unwrap_or_else(|| panic!("no {key}"))["fill"].as_str().unwrap().to_owned()
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn key(t: f64, name: &str) -> Value {
    json!({ "t": t, "type": "key", "name": name })
}

fn chord_comma(t: f64) -> Value {
    // ⌘, on macOS, Ctrl+Shift+, elsewhere.
    json!({ "t": t, "type": "key", "name": ",", "cmd": true, "shift": !cfg!(target_os = "macos") })
}

fn missing(name: &str) -> PathBuf {
    temp(name).join("settings.json")
}

#[test]
fn the_dialog_is_closed_until_asked_for() {
    let t = run(&missing("closed"), vec![]);
    assert!(!open(&t));
    assert!(find(&t, "settings").is_some(), "the gear is in the sidebar");
}

#[test]
fn the_gear_opens_it_with_its_content() {
    let t = run(&missing("gear"), vec![tap(0.3, "settings")]);
    assert!(open(&t));
    let card = find(&t, "settings-dialog").unwrap()["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    assert_eq!(card[2], 520.0, "the card is 520 wide");
    assert_eq!(card[0] + card[2] / 2.0, 855.0, "centred in the window");
    assert!(card[1] < 200.0, "near the top");
    for key in ["settings-esc", "appearance", "settings-done", "version/contr-rune", "version/hc"] {
        assert!(find(&t, key).is_some(), "{key}");
    }
    let values: Vec<&str> = t["elements"].as_array().unwrap().iter().filter_map(|e| e["text"].as_str()).collect();
    assert!(values.contains(&env!("CARGO_PKG_VERSION")), "the crate version is shown");
    assert!(values.contains(&"not installed"), "hc is not installed");
}

#[test]
fn the_chord_opens_it() {
    let t = run(&missing("chord"), vec![chord_comma(0.3)]);
    assert!(open(&t));
}

#[test]
fn esc_done_the_esc_chip_and_the_scrim_close_it() {
    for (name, close) in [
        ("esc key", key(0.6, "Escape")),
        ("done", tap(0.6, "settings-done")),
        ("esc chip", tap(0.6, "settings-esc")),
        ("scrim", tap(0.6, "settings-scrim")),
    ] {
        let t = run(&missing("close"), vec![tap(0.3, "settings"), close]);
        assert!(!open(&t), "{name} closes the dialog");
        assert!(find(&t, "settings-scrim").is_none(), "{name}: the scrim goes too");
    }
}

#[test]
fn light_repaints_the_app_with_the_light_kit_and_dark_brings_it_back() {
    let (dark, light) = (Kit::neutral(true), Kit::neutral(false));
    assert_ne!(dark.c.bg_base.hex(), light.c.bg_base.hex());

    let base = run(&missing("base"), vec![]);
    assert_eq!(fill(&base, "terminal"), dark.c.bg_base.hex().as_str(), "System follows the (dark) headless system");

    let t = run(&missing("light"), vec![tap(0.3, "settings"), tap(0.6, "segment2")]);
    for panel in ["sidebar", "terminal", "files"] {
        assert_eq!(fill(&t, panel), light.c.bg_base.hex().as_str(), "{panel} is light");
    }
    assert_eq!(fill(&t, "window"), light.c.bg_deep.hex().as_str());
    assert!(open(&t), "choosing does not close the dialog");

    let back = run(&missing("back"), vec![tap(0.3, "settings"), tap(0.6, "segment2"), tap(0.9, "segment1")]);
    assert_eq!(fill(&back, "terminal"), dark.c.bg_base.hex().as_str());
}

#[test]
fn the_choice_is_written_and_read_back() {
    let file = missing("persist");
    run(&file, vec![tap(0.3, "settings"), tap(0.6, "segment2")]);
    let text = std::fs::read_to_string(&file).expect("the settings file was written");
    assert!(text.contains("\"light\""), "{text}");

    // A new start reads it: light with no clicks.
    let t = run(&file, vec![]);
    assert_eq!(fill(&t, "terminal"), Kit::neutral(false).c.bg_base.hex().as_str());

    // A broken file falls back to System (dark here).
    std::fs::write(&file, "{ not json").unwrap();
    let t = run(&file, vec![]);
    assert_eq!(fill(&t, "terminal"), Kit::neutral(true).c.bg_base.hex().as_str());
}
