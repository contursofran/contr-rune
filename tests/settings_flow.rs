//! Settings → Agents → Flow (C8): a preset or a control writes `flow` to `agents.json` (a scratch
//! file through `CONTR_AGENTS_CONFIG`; unknown keys stay), the tab shows it again on reopen, and
//! the steppers stop at 1.

use serde_json::Value;

mod common;
use common::*;

const GEAR: &str = "settings";
const AGENTS: &str = "settings-tabs/segment1";
const GENERAL: &str = "settings-tabs/segment0";

/// `agents.json` with a partial flow and a key the app does not know.
const FILE: &str = r#"{"flow":{"workers":1,"merge":"auto"},"theme":{"accent":"blue"}}"#;

fn run(events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").file("CONTR_AGENTS_CONFIG", "agents.json", FILE).events(events).end(2.3).run()
}

/// Whether segment `i` of the control keyed `control` is the chosen one.
fn on(t: &Run, control: &str, i: usize) -> bool {
    t.child(control, &format!("segment{i}")).unwrap_or_else(|| panic!("no segment{i} in {control}"))["itemSelected"] == true
}

fn detail(t: &Run, step: &str) -> String {
    t.find(&format!("flow/step/{step}/detail")).and_then(|e| e["text"].as_str()).unwrap_or_else(|| panic!("no {step}")).to_owned()
}

#[test]
fn the_section_shows_the_flow_of_the_file() {
    let t = run(vec![tap(0.3, GEAR), tap(0.6, AGENTS)]);
    // plan when-needed and review code (the defaults) are the "Build and review" preset.
    assert!(on(&t, "flow-presets", 1) && !on(&t, "flow-presets", 0));
    assert_eq!(detail(&t, "Planner"), "when needed");
    assert_eq!(detail(&t, "Merge"), "when CI and review pass");
    assert_eq!(t.find("flow/workers/value").unwrap()["text"], "1");
    assert!(t.has("flow/review/note"));
}

#[test]
fn a_preset_is_written_to_agents_json_and_shown_again_on_reopen() {
    // Builder only: plan never, review none.
    let t = run(vec![tap(0.3, GEAR), tap(0.6, AGENTS), tap(0.9, "flow-presets/segment0"), tap(1.2, GENERAL), tap(1.5, AGENTS)]);
    let saved: Value = serde_json::from_str(t.file("agents.json")).unwrap();
    assert_eq!((saved["flow"]["plan"].as_str(), saved["flow"]["review"].as_str()), (Some("never"), Some("none")));
    assert_eq!((saved["flow"]["workers"].as_i64(), saved["flow"]["merge"].as_str()), (Some(1), Some("auto")), "the other values stay");
    assert_eq!(saved["theme"]["accent"], "blue", "unknown keys stay");
    // After the tab left and opened again, the section reads the file.
    assert!(on(&t, "flow-presets", 0));
    assert_eq!((detail(&t, "Planner"), detail(&t, "Reviewer")), ("off".to_owned(), "off".to_owned()));
    assert!(!t.has("flow/review/note"));
}

#[test]
fn the_controls_write_one_value_and_the_steppers_stop_at_1() {
    let t = run(vec![
        tap(0.3, GEAR),
        tap(0.6, AGENTS),
        tap(0.9, "flow-merge/segment0"),
        tap(1.2, "flow-review/segment2"),
        tap(1.5, "flow/workers/minus"),
        tap(1.8, "flow/debugger_after/plus"),
    ]);
    let saved: Value = serde_json::from_str(t.file("agents.json")).unwrap();
    let f = &saved["flow"];
    assert_eq!((f["merge"].as_str(), f["review"].as_str(), f["plan"].as_str()), (Some("ask"), Some("every"), Some("when-needed")));
    assert_eq!((f["workers"].as_i64(), f["debugger_after"].as_i64()), (Some(1), Some(3)), "workers stay at 1");
    assert!(!on(&t, "flow-presets", 0) && !on(&t, "flow-presets", 1) && !on(&t, "flow-presets", 2));
    assert!(!t.has("flow/error"));
}
