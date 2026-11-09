//! The update check (R3a): the pill in the title bar, the toast, the pill's popover and the Settings
//! row, driven by a release fixture through `CONTR_UPDATE_URL` (a `file://` URL; no network). Keys:
//! `update-pill`, `update-toast`, `update-panel`, `update-panel/restart|later`,
//! `version/contr-rune/value`, `settings-check-updates`.

use std::path::Path;

use serde_json::Value;

mod common;
use common::*;

fn fixture(name: &str) -> String {
    format!("file://{}", Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update").join(name).display())
}

fn run(release: &str, events: Vec<Value>) -> Run {
    App::new().scene("update").env("CONTR_UPDATE_URL", fixture(release)).events(events).end(1.0).run()
}

#[test]
fn a_newer_release_shows_the_pill_and_one_toast() {
    let t = run("newer.json", vec![]);
    let pill = t.rect("update-pill");
    assert!(pill[0] < t.rect("split")[0], "the pill is left of the split button");
    assert!(t.texts().iter().any(|x| x == "Restart to update"));
    assert!(t.has("update-toast"));
    assert!(t.texts().iter().any(|x| x == "contr-rune 9.9.9 is ready. Restart to finish updating."));
    assert!(!t.has("update-panel"), "the popover waits for a click");
}

#[test]
fn the_current_release_shows_nothing() {
    let t = run("current.json", vec![]);
    assert!(!t.has("update-pill") && !t.has("update-toast"));
}

#[test]
fn the_toast_closes_for_good() {
    let r = run("newer.json", vec![]).rect_in("update-toast", "close");
    let tap_close = serde_json::json!({ "t": 0.3, "type": "tap", "x": r[0] + r[2] / 2.0, "y": r[1] + r[3] / 2.0 });
    let t = run("newer.json", vec![tap_close]);
    assert!(!t.has("update-toast"));
    assert!(t.has("update-pill"), "the pill stays");
}

#[test]
fn the_pill_opens_the_popover_with_the_release_notes() {
    let t = run("newer.json", vec![tap(0.3, "update-pill")]);
    assert!(t.has("update-panel") && t.has("update-panel/restart") && t.has("update-panel/later"));
    let texts = t.texts();
    for want in ["contr-rune 9.9.9 is ready", "What's new", "Update pill and toast", "Fix: tabs keep their order"] {
        assert!(texts.iter().any(|x| x == want), "{want}");
    }
    assert!(texts.iter().any(|x| x.starts_with("You are on ") && x.contains("Agents keep running in hc")));
    assert!(!t.has("update-panel/item/3"), "three items in the fixture");
    // The pill is the popover's anchor: it sits under it, inside the window.
    let (pill, panel) = (t.rect("update-pill"), t.rect("update-panel"));
    assert!(panel[1] >= pill[1] + pill[3] && panel[0] + panel[2] <= 1710.0);
}

#[test]
fn later_closes_the_popover_and_restart_now_closes_both() {
    let later = run("newer.json", vec![tap(0.3, "update-pill"), tap(0.6, "update-panel/later")]);
    assert!(!later.has("update-panel") && later.has("update-pill"));
    let restart = run("newer.json", vec![tap(0.3, "update-pill"), tap(0.6, "update-panel/restart")]);
    assert!(!restart.has("update-panel") && !restart.has("update-toast"));
}

#[test]
fn settings_says_which_version_is_ready() {
    let newer = run("newer.json", vec![tap(0.3, "settings"), tap(0.5, "settings-tabs/segment2")]);
    assert_eq!(newer.text_in("version/contr-rune", "version/contr-rune/value").unwrap(), format!("{} \u{b7} 9.9.9 available", env!("CARGO_PKG_VERSION")));
    assert!(newer.has("settings-check-updates"));
    let current = run("current.json", vec![tap(0.3, "settings"), tap(0.5, "settings-tabs/segment2")]);
    assert_eq!(current.text_in("version/contr-rune", "version/contr-rune/value").unwrap(), format!("{} \u{b7} up to date \u{b7} checked just now", env!("CARGO_PKG_VERSION")));
}

/// The key pair of the fixtures' signatures (made for tests; the private half is not kept).
const TEST_KEY: &str = "d7fa0b684898d1703f79456a4923aac56a1e5af9f1928d77d337d58179a9c7cf";

/// "Restart now" with the assets of `dir` (next to its `newer.json`), a dry run (no swap), then Settings.
fn install(dir: &str) -> Run {
    let events = vec![tap(0.3, "update-pill"), tap(0.6, "update-panel/restart"), tap(0.9, "settings"), tap(1.1, "settings-tabs/segment2")];
    App::new()
        .scene("update")
        .env("CONTR_UPDATE_URL", fixture(&format!("../{dir}/newer.json")))
        .env("CONTR_UPDATE_PUBKEY", TEST_KEY)
        .env("CONTR_UPDATE_DRY", "1")
        .events(events)
        .end(1.8)
        .run()
}

fn version_value(t: &Run) -> String {
    t.text_in("version/contr-rune", "version/contr-rune/value").unwrap()
}

#[test]
fn restart_now_downloads_and_verifies_the_signed_zip_then_stops() {
    let t = install("update");
    assert!(version_value(&t).contains("9.9.9 verified (dry run)"), "{}", version_value(&t));
    assert!(!t.has("update/error"));
}

#[test]
fn a_bad_signature_refuses_the_update_and_shows_in_settings() {
    let t = install("update-bad");
    assert!(t.texts().iter().any(|x| x.contains("signature does not match")), "{:?}", t.texts());
    assert!(t.has("update/error"));
    assert!(!t.has("update-toast"), "an error is not a toast (D176)");
    assert!(version_value(&t).contains("9.9.9 available"));
}
