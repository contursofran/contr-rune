//! hc in Settings (H1): the dialog's "hc" row shows the core version from the fake core
//! (`CONTR_HC_FAKE`), "not installed" in a headless run without it, "off" for `CONTR_HC=off`, and
//! "not running: <error>" when the core fails. No test dials a real hc.

mod common;

use common::*;
use serde_json::json;

/// The hc row of the settings scene.
fn hc_value(app: App) -> String {
    let r = app.scene("settings").end(0.5).run();
    r.find("version/hc/value").expect("version/hc/value")["text"].as_str().unwrap().to_owned()
}

#[test]
fn the_fake_core_version_is_shown_in_settings() {
    assert_eq!(hc_value(App::new().fixture("basic.json")), "0.1.19");
}

#[test]
fn a_headless_run_without_the_fake_does_not_dial_hc() {
    // Even with a real hc on this machine: the value is never a version.
    assert_eq!(hc_value(App::new()), "not installed");
}

#[test]
fn hc_off_is_shown_as_off() {
    assert_eq!(hc_value(App::new().env("CONTR_HC", "off")), "off");
}

#[test]
fn a_core_that_fails_or_has_no_fixture_is_shown_as_not_running_with_its_error() {
    let failing = App::new().fixture_with(|_| json!({"errors": {"server.status": {"kind": "unavailable", "message": "core is starting"}}}));
    assert_eq!(hc_value(failing), "not running: core is starting");
    let missing = hc_value(App::new().env("CONTR_HC_FAKE", "/nonexistent/fixture.json"));
    assert!(missing.starts_with("not running: fake core fixture"), "{missing}");
}
