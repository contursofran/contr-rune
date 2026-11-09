//! Live agent state (H6), against the in-process fake core (`CONTR_HC_FAKE`, fixture
//! `tests/fixtures/hc/blocked.json`: an orchestrator `t3` and two idle workers `t18`, `t19`; at
//! 100 ms `t18` starts working and at 200 ms `t19` is blocked). The `glyph` of each
//! `agent/<address>` row says its state in its label: `working`, `waiting`, `needs you` or
//! `idle`. Needs-you is the only red; its row carries the message as help. No test dials a real hc.

mod common;

use std::sync::OnceLock;

use common::*;
use serde_json::{Value, json};

fn status(state: &str, kind: Option<&str>, msg: &str) -> Value {
    json!({"state": state, "kind": kind, "msg": msg, "title": "", "app": "claude-code", "progress": null})
}

fn set_status(after_ms: u64, seq: u64, terminal: &str, st: Value) -> Value {
    hc_event(after_ms, seq, "status", terminal, json!({"status": st, "records": {}, "reported": true}))
}

/// The fixture with its `events` replaced (`None` keeps its own) and the folder set. The events
/// are in by 350 ms of script time, so t = 1 leaves them time.
fn run(events: Option<Vec<Value>>) -> Run {
    App::new()
        .project("contr")
        .scene("agents")
        .fixture_with(move |d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/blocked.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            if let Some(e) = events {
                fx["events"] = json!(e);
            }
            fx
        })
        .end(1.0)
        .run()
}

/// The launch with the fixture's own events: shared by the tests that read it.
fn calm() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(None))
}

/// The rows of the agents scene.
trait Agents {
    fn row(&self, address: &str) -> Option<&Value>;
    fn glyph(&self, address: &str) -> &Value;
    fn state(&self, address: &str) -> String;
    fn subtitle(&self, address: &str) -> Option<String>;
    fn rows(&self) -> Vec<String>;
}

impl Agents for Run {
    fn row(&self, address: &str) -> Option<&Value> {
        let key = format!("agent/{address}");
        self.find(&key)
    }

    /// The `glyph` inside the row of `address`.
    fn glyph(&self, address: &str) -> &Value {
        let marker = format!("/agent/{address}/");
        self.elements()
            .iter()
            .find(|e| e["key"] == "glyph" && e["path"].as_str().is_some_and(|p| p.contains(&marker)))
            .unwrap_or_else(|| panic!("no glyph in agent/{address}"))
    }

    fn state(&self, address: &str) -> String {
        self.glyph(address)["label"].as_str().unwrap_or_else(|| panic!("glyph of {address} has no label: {}", self.glyph(address))).to_owned()
    }

    fn subtitle(&self, address: &str) -> Option<String> {
        self.text_in(&format!("agent/{address}"), "subtitle")
    }

    fn rows(&self) -> Vec<String> {
        let mut rows: Vec<(f64, String)> = self
            .elements()
            .iter()
            .filter_map(|e| e["key"].as_str().filter(|k| k.starts_with("agent/")).map(|k| (e["rect"][1].as_f64().unwrap(), k.trim_start_matches("agent/").to_owned())))
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|r| r.1).collect()
    }
}

#[test]
fn events_turn_one_worker_working_and_another_needs_you() {
    let r = calm();
    assert_eq!(r.rows(), vec!["t3", "t18", "t19"]);
    assert_eq!(r.state("t18"), "working");
    assert_eq!(r.state("t19"), "needs you");
    // The orchestrator is idle itself but a live child works: it waits, and its meta has no `idle`.
    assert_eq!(r.state("t3"), "waiting");
    assert_eq!(r.subtitle("t3").as_deref(), Some("opus"));
    assert_eq!(r.subtitle("t18").as_deref(), Some("b-242  sonnet"));
}

#[test]
fn needs_you_is_red_and_nothing_else_is() {
    let r = calm();
    // The ink of a glyph: its own colour, else the colour of the first text inside it (the braille frames).
    let color = |a: &str| -> (u8, u8, u8) {
        let marker = format!("/agent/{a}/");
        let own = r.glyph(a)["color"].as_str();
        let inner = r.elements().iter().find(|e| e["text"].is_string() && e["path"].as_str().is_some_and(|p| p.contains(&marker) && p.contains("glyph/"))).and_then(|e| e["color"].as_str());
        let hex = own.or(inner).unwrap_or_else(|| panic!("no colour in {a}")).trim_start_matches('#').to_owned();
        let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        (c(0), c(2), c(4))
    };
    let red = |(r, g, b): (u8, u8, u8)| r > g.saturating_add(60) && r > b.saturating_add(60);
    assert!(red(color("t19")), "needs-you is red: {:?}", color("t19"));
    assert!(!red(color("t18")), "working is not red");
    assert_ne!(color("t3"), color("t19"), "the Claude mark keeps its own colour while it waits");
}

#[test]
fn a_needs_you_row_carries_its_message_as_help() {
    let r = calm();
    let help = |a: &str| r.row(a).and_then(|e| e["help"].as_str().map(str::to_owned));
    assert_eq!(help("t19").as_deref(), Some("Run cargo test?"));
    assert_eq!(help("t18"), None);
}

#[test]
fn an_orchestrator_with_no_working_child_is_idle() {
    let r = run(Some(vec![set_status(100, 11, "t18", status("working", None, "")), set_status(200, 12, "t18", status("idle", None, ""))]));
    assert_eq!(r.state("t18"), "idle");
    assert_eq!(r.state("t3"), "idle");
    assert_eq!(r.subtitle("t3").as_deref(), Some("opus  idle"));
}

#[test]
fn a_missed_snapshot_replaces_the_state_and_older_events_are_dropped() {
    let snapshot: Value = serde_json::from_str::<Value>(include_str!("fixtures/hc/blocked.json")).unwrap()["terminals"].clone();
    let mut terminals = snapshot.as_array().unwrap().clone();
    // In the snapshot t18 is idle and t19 is working.
    terminals[2]["status"] = status("working", None, "");
    let missed = json!({"after_ms": 250, "method": "missed", "params": {"seq": 20, "terminals": terminals}});
    let r = run(Some(vec![
        set_status(100, 11, "t18", status("working", None, "")),
        set_status(150, 12, "t19", status("blocked", Some("permission"), "Run cargo test?")),
        missed,
        // Behind the snapshot (seq 20): dropped.
        set_status(300, 15, "t19", status("blocked", Some("permission"), "stale")),
        // After it: applied.
        set_status(350, 21, "t18", status("blocked", Some("auth"), "Log in")),
    ]));
    assert_eq!(r.state("t19"), "working", "the snapshot won over the older blocked event");
    assert_eq!(r.state("t18"), "needs you", "an event after the snapshot applies");
}

#[test]
fn an_ended_agent_has_no_row_and_the_orchestrator_stops_waiting() {
    let r = run(Some(vec![
        set_status(100, 11, "t18", status("working", None, "")),
        hc_event(200, 12, "finished", "t18", json!({"reason": "exited", "exit_code": 0, "signal": null, "finished_ms": 1})),
    ]));
    assert_eq!(r.rows(), vec!["t3", "t19"]);
    assert_eq!(r.state("t3"), "idle");
}

#[test]
fn a_tab_whose_program_is_blocked_shows_needs_attention() {
    // The scene opens one tab: the fake core makes it `t20`. A blocked status for it marks its sidebar row.
    // The kit's attention mark is a 7 point dot in the status slot.
    let dot = |r: &Run| r.elements().iter().any(|e| e["path"].as_str().is_some_and(|p| p.contains("/term-1/status/status-mark/")) && e["rect"][2] == 7.0 && e["rect"][3] == 7.0);
    let r = run(Some(vec![set_status(250, 13, "t20", status("blocked", Some("permission"), "Run it?"))]));
    assert!(dot(&r), "the blocked tab has the attention dot");
    assert!(!dot(calm()), "a calm tab has none");
}

#[test]
fn a_missed_snapshot_without_the_tab_terminal_closes_the_tab() {
    // The scene opens one tab (`t20`). A `missed` snapshot that lacks it must close it, as `removed` would.
    let fx: Value = serde_json::from_str(include_str!("fixtures/hc/blocked.json")).unwrap();
    let missed = json!({"after_ms": 250, "method": "missed", "params": {"seq": 20, "terminals": fx["terminals"]}});
    let r = run(Some(vec![missed]));
    assert!(!r.has("term-1"), "the tab row is gone");
    assert!(calm().elements().iter().any(|e| e["key"] == "term-1"), "without the snapshot the tab stays");
}
