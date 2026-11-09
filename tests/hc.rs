//! hc in Settings (H1): the dialog's "hc" row shows the core version from the fake core
//! (`CONTR_HC_FAKE`), "not installed" in a headless run without it, "off" for `CONTR_HC=off`, and
//! "not running: <error>" when the core fails. No test dials a real hc.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

fn temp(name: &str) -> PathBuf {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("contr-rune-hc-{name}-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc").join(name)
}

/// Run the settings scene headless with the given hc environment; return the tree.
fn run(envs: &[(&str, &Path)], off: bool) -> Value {
    let dir = temp("run");
    let out = dir.join("tree.json");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.env_remove("CONTR_HC_FAKE")
        .env_remove("CONTR_HC")
        .env("CONTR_SETTINGS", dir.join("settings.json"))
        .env("CONTR_PROJECTS", std::env::temp_dir())
        .args(["--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "settings"]);
    if off {
        cmd.env("CONTR_HC", "off");
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let run = cmd.output().expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    v
}

fn hc_value(tree: &Value) -> String {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == "version/hc/value").expect("version/hc/value")["text"].as_str().unwrap().to_owned()
}

#[test]
fn the_fake_core_version_is_shown_in_settings() {
    let t = run(&[("CONTR_HC_FAKE", &fixture("basic.json"))], false);
    assert_eq!(hc_value(&t), "0.1.19");
}

#[test]
fn the_agents_fixture_connects_too() {
    let t = run(&[("CONTR_HC_FAKE", &fixture("agents.json"))], false);
    assert_eq!(hc_value(&t), "0.1.19");
}

#[test]
fn a_headless_run_without_the_fake_does_not_dial_hc() {
    // Even with a real hc on this machine: the value is never a version.
    let t = run(&[], false);
    assert_eq!(hc_value(&t), "not installed");
}

#[test]
fn hc_off_is_shown_as_off() {
    let t = run(&[], true);
    assert_eq!(hc_value(&t), "off");
}

#[test]
fn a_core_that_fails_is_shown_as_not_running_with_its_error() {
    let dir = temp("failing");
    let fx = dir.join("failing.json");
    std::fs::write(&fx, json!({"errors": {"server.status": {"kind": "unavailable", "message": "core is starting"}}}).to_string()).unwrap();
    let t = run(&[("CONTR_HC_FAKE", &fx)], false);
    assert_eq!(hc_value(&t), "not running: core is starting");
}

#[test]
fn a_missing_fixture_is_shown_as_not_running() {
    let t = run(&[("CONTR_HC_FAKE", Path::new("/nonexistent/fixture.json"))], false);
    assert!(hc_value(&t).starts_with("not running: fake core fixture"), "{}", hc_value(&t));
}
