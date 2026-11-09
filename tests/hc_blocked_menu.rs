//! A2: a blocked agent's menu in the questions card, against the in-process fake core
//! (`tests/fixtures/hc/agents.json` with a screen for the blocked agent `t18`; log in
//! `CONTR_HC_FAKE_LOG`). The card's first tab is `b-242` (`t18`).

mod common;

use common::*;
use serde_json::{Value, json};

const MENU: &str = "● Running tests\n\n╭──────────────────────────────╮\n│ Bash command                 │\n│   cargo test                 │\n│                              │\n│ Do you want to proceed?      │\n│ ❯ 1. Yes                     │\n│   2. Yes, and don't ask again │\n│   3. No, and tell Claude what to do differently │\n╰──────────────────────────────╯\n";

fn run(screen: &str, events: Vec<Value>) -> Run {
    run_with(screen, events, vec![])
}

/// `fx_events` are the core's own events, on its clock.
fn run_with(screen: &str, events: Vec<Value>, fx_events: Vec<Value>) -> Run {
    let screen = screen.to_owned();
    App::new()
        .project("contr")
        .fixture_with(move |d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx["screens"] = json!({"t18": screen});
            fx["events"] = json!(fx_events);
            fx
        })
        .file("CONTR_OWNER_FILE", "owner.json", include_str!("fixtures/hc/owner.json"))
        .events(events)
        .end(4.0)
        .run()
}

fn keys(r: &Run) -> Vec<&Value> {
    r.calls("terminal.keys").into_iter().map(|l| &l["params"]).collect()
}

fn text(r: &Run, key: &str) -> String {
    r.find(key).unwrap_or_else(|| panic!("no element {key}"))["text"].as_str().unwrap_or("").to_owned()
}

#[test]
fn the_blocked_agents_screen_shows_as_a_menu() {
    let r = run(MENU, vec![tap(0.5, "notch")]);
    let reads = r.calls("terminal.read");
    assert_eq!(reads.len(), 1, "one read when the card opens");
    assert_eq!((&reads[0]["params"]["terminal"], &reads[0]["params"]["lines"]), (&json!("t18"), &json!(40)));
    assert_eq!(text(&r, "qcard/prompt/t18/question"), "Do you want to proceed?");
    assert_eq!(text(&r, "qcard/prompt/t18/detail"), "Bash command\ncargo test");
    for (n, label) in [(1, "Yes"), (2, "Yes, and don't ask again"), (3, "No, and tell Claude what to do differently")] {
        assert_eq!(r.find(&format!("qcard/choice/t18/{n}")).unwrap()["label"], label);
    }
    assert!(r.find("qcard/choice/t18/4").is_none());
    assert!(r.find("qcard/open/t18").is_some(), "Open terminal stays");
    assert_eq!(text(&r, "qcard/keys"), "1-3 send  \u{23ce} next  esc close");
    assert!(keys(&r).is_empty(), "looking sends nothing");
}

#[test]
fn a_digit_sends_that_choice_as_a_key_without_enter() {
    let r = run(MENU, vec![tap(0.5, "notch"), typed(1.5, "2")]);
    assert_eq!(keys(&r), vec![&json!({"terminal": "t18", "keys": ["2"]})]);
    assert_eq!(r.calls("terminal.read").len(), 2, "the screen is read again before the key goes");
    assert!(r.find("qcard/error").is_none());
    assert!(r.has("qcard"), "the card stays; the agent's next status moves it on");
}

#[test]
fn a_click_sends_the_choice() {
    let r = run(MENU, vec![tap(0.5, "notch"), tap(1.5, "qcard/choice/t18/1")]);
    assert_eq!(keys(&r), vec![&json!({"terminal": "t18", "keys": ["1"]})]);
}

#[test]
fn a_choice_that_asks_for_text_opens_the_terminal_and_sends_nothing() {
    let r = run(MENU, vec![tap(0.5, "notch"), typed(1.5, "3")]);
    assert!(keys(&r).is_empty());
    assert!(!r.has("qcard"), "the card closes to show the agent's terminal");
}

#[test]
fn an_unknown_screen_leaves_only_the_open_terminal_button() {
    let r = run("just some output\n", vec![tap(0.5, "notch"), typed(1.5, "1")]);
    assert!(r.find("qcard/prompt/t18").is_none() && r.find("qcard/choice/t18/1").is_none());
    assert!(r.find("qcard/open/t18").is_some());
    assert_eq!(text(&r, "qcard/keys"), "\u{23ce} next  esc close");
    assert!(keys(&r).is_empty(), "a digit with no menu sends nothing");
}

#[test]
fn the_agents_next_status_reads_the_screen_again() {
    let blocked = |msg: &str| json!({"status": {"state": "blocked", "kind": "permission", "msg": msg, "title": "", "app": "claude-code", "progress": null}, "records": {}, "reported": true});
    let r = run_with(MENU, vec![tap(0.5, "notch")], vec![hc_event(1500, 1, "status", "t18", blocked("Run cargo build?"))]);
    assert_eq!(r.calls("terminal.read").len(), 2, "once on opening, once for the new message");
    assert!(r.find("qcard/prompt/t18").is_some());
}
