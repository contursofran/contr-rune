//! The Settings tabs and Agents → Roles (C7): the General / Agents / Updates tabs, the Roles with
//! their selects and argv lines, a pick written to `agents.json` (a scratch file through
//! `CONTR_AGENTS_CONFIG`; unknown keys stay), and the missing orchestrator with its suggestion.

use serde_json::Value;

mod common;
use common::*;

const GEAR: &str = "settings";
const AGENTS: &str = "settings-tabs/segment1";
const UPDATES: &str = "settings-tabs/segment2";
const SUGGESTION: &str = "not set \u{b7} suggested: claude --model opus --effort medium --permission-mode auto";

/// `agents.json` with a planner profile and a key the app does not know.
const FILE: &str = r#"{"profiles":{"planner":{"argv":["claude","--model","opus","--effort","low","--verbose"]}},"theme":{"accent":"blue"}}"#;

fn run(file: &str, events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").file("CONTR_AGENTS_CONFIG", "agents.json", file).events(events).end(1.5).run()
}

fn argv(t: &Run, role: &str) -> String {
    t.find(&format!("role/{role}/argv")).and_then(|e| e["text"].as_str()).unwrap_or_else(|| panic!("no argv of {role}")).to_owned()
}

#[test]
fn the_tabs_switch_the_body() {
    let general = run(FILE, vec![tap(0.3, GEAR)]);
    assert!(general.has("settings-tabs") && general.has("appearance"));
    assert!(!general.has("version/contr-rune") && !general.has("role/planner/model"));

    let agents = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS)]);
    assert!(agents.has("role/planner/model") && !agents.has("appearance") && !agents.has("version/contr-rune"));

    let updates = run(FILE, vec![tap(0.3, GEAR), tap(0.6, UPDATES)]);
    for key in ["version/contr-rune", "version/hc", "settings-check-updates"] {
        assert!(updates.has(key), "{key} is on Updates");
    }
    assert!(!updates.has("appearance") && !updates.has("role/planner/model"));
}

#[test]
fn the_roles_show_their_selects_and_argv_lines() {
    let t = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS)]);
    for role in ["orchestrator", "planner", "builder", "reviewer", "debugger"] {
        for f in ["cli", "model", "effort", "argv"] {
            assert!(t.has(&format!("role/{role}/{f}")), "role/{role}/{f}");
        }
    }
    assert_eq!(argv(&t, "planner"), "claude --model opus --effort low --verbose", "the file's profile");
    assert_eq!(argv(&t, "builder"), "claude --model sonnet --effort medium --permission-mode auto", "the default");
    let texts = t.texts();
    assert!(texts.iter().any(|x| x == "Claude Code") && texts.iter().any(|x| x == "sonnet"));
}

#[test]
fn a_pick_is_written_to_agents_json_and_keeps_the_rest() {
    // The model list is opus, sonnet, haiku: option1 is sonnet.
    let t = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS), tap(0.9, "role/planner/model-trigger"), tap(1.1, "option1")]);
    assert_eq!(argv(&t, "planner"), "claude --model sonnet --effort low --verbose");
    let saved: Value = serde_json::from_str(t.file("agents.json")).unwrap();
    assert_eq!(saved["profiles"]["planner"]["argv"], serde_json::json!(["claude", "--model", "sonnet", "--effort", "low", "--verbose"]));
    assert_eq!(saved["theme"]["accent"], "blue", "unknown keys stay");
    assert!(saved["profiles"].get("builder").is_none(), "an untouched default is not frozen into the file");
    assert!(!t.has("role/planner/model-content"), "the list closes after a pick");
}

#[test]
fn a_missing_orchestrator_shows_the_suggestion_and_set_creates_it() {
    let t = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS)]);
    assert_eq!(argv(&t, "orchestrator"), SUGGESTION);
    assert!(t.has("role/orchestrator/set") && !t.has("role/planner/set"));

    let set = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS), tap(0.9, "role/orchestrator/set")]);
    assert_eq!(argv(&set, "orchestrator"), "claude --model opus --effort medium --permission-mode auto");
    assert!(!set.has("role/orchestrator/set"));
    let saved: Value = serde_json::from_str(set.file("agents.json")).unwrap();
    assert_eq!(saved["profiles"]["orchestrator"]["argv"], serde_json::json!(["claude", "--model", "opus", "--effort", "medium", "--permission-mode", "auto"]));
    assert_eq!(saved["profiles"]["planner"]["argv"][4], "low", "the other profiles stay");
}

#[test]
fn a_pick_on_a_missing_orchestrator_creates_it_from_the_suggestion() {
    // The effort list is low, medium, high, xhigh, max: option2 is high.
    let t = run(FILE, vec![tap(0.3, GEAR), tap(0.6, AGENTS), tap(0.9, "role/orchestrator/effort-trigger"), tap(1.1, "option2")]);
    assert_eq!(argv(&t, "orchestrator"), "claude --model opus --effort high --permission-mode auto");
}

#[test]
fn a_broken_file_shows_the_error_and_a_pick_does_not_overwrite_it() {
    let t = run("{ broken", vec![tap(0.3, GEAR), tap(0.6, AGENTS), tap(0.9, "role/builder/model-trigger"), tap(1.1, "option0")]);
    assert!(t.has("roles/error"));
    assert_eq!(t.file("agents.json"), "{ broken");
}
