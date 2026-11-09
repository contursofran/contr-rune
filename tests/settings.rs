//! The settings dialog and the light theme (K7a): the gear and ⌘, open the dialog; esc, Done and
//! the scrim close it; choosing Light repaints the app with `Kit::neutral(false)` and the choice
//! lives in the settings file (`CONTR_SETTINGS` points it at a temp file here).

use std::sync::OnceLock;

use rune_kit::Kit;
use serde_json::Value;

mod common;
use common::*;

fn run(events: Vec<Value>) -> Run {
    // Ends 1 s after the last tap or more: the theme change fades in.
    App::new().scene("three-tabs").events(events).end(2.0).run()
}

/// The launch with no events (the dialog closed, the System theme), shared by the tests that read it.
fn start() -> &'static Run {
    static START: OnceLock<Run> = OnceLock::new();
    START.get_or_init(|| run(vec![]))
}

fn open(t: &Run) -> bool {
    t.has("settings-dialog")
}

fn fill(t: &Run, key: &str) -> String {
    t.find(key).unwrap_or_else(|| panic!("no {key}"))["fill"].as_str().unwrap().to_owned()
}

#[test]
fn the_dialog_is_closed_until_asked_for() {
    assert!(!open(start()));
}

#[test]
fn the_gear_opens_it_with_its_content() {
    let t = run(vec![tap(0.3, "settings")]);
    assert!(open(&t));
    let card = t.rect("settings-dialog");
    assert_eq!(card[2], 520.0, "the card is 520 wide");
    assert_eq!(card[0] + card[2] / 2.0, 855.0, "centred in the window");
    assert!(card[1] < 200.0, "near the top");
    for key in ["settings-esc", "appearance", "settings-done", "version/contr-rune", "version/hc"] {
        assert!(t.has(key), "{key}");
    }
    let values = t.texts();
    assert!(values.iter().any(|v| v == env!("CARGO_PKG_VERSION")), "the crate version is shown");
    assert!(values.iter().any(|v| v == "not installed"), "hc is not installed");
}

#[test]
fn the_chord_opens_it() {
    let t = run(vec![chord(0.3, ",", false)]);
    assert!(open(&t));
}

#[test]
fn esc_done_the_esc_chip_and_the_scrim_close_it() {
    for (name, close) in [("esc key", key(0.6, "Escape")), ("done", tap(0.6, "settings-done")), ("esc chip", tap(0.6, "settings-esc")), ("scrim", tap(0.6, "settings-scrim"))] {
        let t = run(vec![tap(0.3, "settings"), close]);
        assert!(!open(&t), "{name} closes the dialog");
        assert!(!t.has("settings-scrim"), "{name}: the scrim goes too");
    }
}

#[test]
fn light_repaints_the_app_with_the_light_kit_and_dark_brings_it_back() {
    let (dark, light) = (Kit::neutral(true), Kit::neutral(false));
    assert_ne!(dark.c.bg_base.hex(), light.c.bg_base.hex());

    assert_eq!(fill(start(), "terminal"), dark.c.bg_base.hex().as_str(), "System follows the (dark) headless system");

    let t = run(vec![tap(0.3, "settings"), tap(0.6, "segment2")]);
    for panel in ["sidebar", "terminal", "files"] {
        assert_eq!(fill(&t, panel), light.c.bg_base.hex().as_str(), "{panel} is light");
    }
    assert_eq!(fill(&t, "window"), light.c.bg_deep.hex().as_str());
    assert!(open(&t), "choosing does not close the dialog");

    let back = run(vec![tap(0.3, "settings"), tap(0.6, "segment2"), tap(0.9, "segment1")]);
    assert_eq!(fill(&back, "terminal"), dark.c.bg_base.hex().as_str());
}

#[test]
fn the_choice_is_written_and_read_back() {
    // The broken-file fallback is the unit test in `src/settings.rs`.
    let first = App::new().scene("three-tabs").file("CONTR_SETTINGS", "settings.json", "").events(vec![tap(0.3, "settings"), tap(0.6, "segment2")]).end(2.0).run();
    let text = first.file("settings.json");
    assert!(text.contains("\"light\""), "{text}");

    // A new start reads it: light with no clicks.
    let again = App::new().scene("three-tabs").file("CONTR_SETTINGS", "settings.json", text).end(2.0).run();
    assert_eq!(fill(&again, "terminal"), Kit::neutral(false).c.bg_base.hex().as_str());
}
