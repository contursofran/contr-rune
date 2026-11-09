//! The orchestrator outline (H10), against the in-process fake core (`CONTR_HC_FAKE`) and a temp
//! `owner.json` (`CONTR_OWNER_FILE`). The project `contr` has the open to-dos T1 and T2 (T2 has a
//! second line), T3 is done and T4 belongs to another project. Tapping `agent/t3` shows the
//! orchestrator's terminal: ticks T1 and T2 on its right edge. A plain terminal has no outline. No test
//! dials a real hc or reads the owner's real file.

mod common;

use common::*;
use serde_json::{Value, json};

const OWNER: &str = r#"{
  "todos": [
    {"id": "T1", "project": "contr", "coordinator": "t3", "from": "b-242", "kind": "do", "text": "Renew the certificate", "pinned": "2026-11-08T14:38:00-03:00", "done": "0001-01-01T00:00:00Z"},
    {"id": "T2", "project": "contr", "coordinator": "t3", "from": "b-241", "kind": "know", "text": "CI failed on #329\nFlaky hc test; I reran it and it passed.", "pinned": "2026-11-08T15:02:00-03:00"},
    {"id": "T3", "project": "contr", "kind": "do", "text": "Old news", "done": "2026-11-02T10:00:00Z"},
    {"id": "T4", "project": "elsewhere", "kind": "do", "text": "Not here"}
  ]
}"#;

fn run(events: Vec<Value>) -> Run {
    App::new()
        .project("contr")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx
        })
        .file("CONTR_OWNER_FILE", "owner.json", OWNER)
        .events(events)
        .end(2.0)
        .run()
}

#[test]
fn ticks_sit_on_the_right_edge_of_the_orchestrator_terminal_only() {
    let r = run(vec![tap(0.5, "agent/t3")]);
    let panel = r.rect("terminal");
    let (t1, t2) = (r.rect("outline/tick/T1"), r.rect("outline/tick/T2"));
    assert!(!r.has("outline/tick/T3") && !r.has("outline/tick/T4"), "only the open to-dos of its project");
    assert_eq!((t1[2], t1[3]), (20.0, 10.0));
    assert_eq!(t1[0] + t1[2], panel[0] + panel[2] - 10.0, "inset 10 from the panel's right edge");
    assert_eq!(t2[1] - t1[1], 11.0, "oldest first, pitch 11");
    let column = r.rect("outline");
    assert!((column[1] + column[3] / 2.0 - (panel[1] + panel[3] / 2.0)).abs() <= 1.0, "centred on the height");
    assert_eq!(r.find("outline").unwrap()["label"], "Orchestrator messages, 0 new");
    assert!(!r.has("outline/count"), "nothing is new while the owner looks");
    assert!(!r.has("outline/card"));
    // The plain terminal (tab `t-2`) has none.
    assert!(!run(vec![tap(0.5, "agent/t3"), tap(0.8, "t-2")]).has("outline"));
}

#[test]
fn hovering_a_tick_shows_its_card_at_the_left() {
    let r = run(vec![tap(0.5, "agent/t3"), hover(1.0, "outline/tick/T2")]);
    assert_eq!(r.find("outline/card/title").unwrap()["text"], "CI failed on #329");
    assert_eq!(r.find("outline/card/detail").unwrap()["text"], "Flaky hc test; I reran it and it passed.");
    assert_eq!(r.find("outline/card/time").unwrap()["text"], "15:02");
    let (card, panel, tick) = (r.rect("outline/card"), r.rect("terminal"), r.rect("outline/tick/T2"));
    assert_eq!(card[2], 380.0);
    assert_eq!(card[0] + card[2], panel[0] + panel[2] - 40.0, "40 from the panel's right edge");
    assert!((card[1] + card[3] / 2.0 - (tick[1] + tick[3] / 2.0)).abs() <= 1.0, "centred on the tick");
    // A to-do with one line: kind and sender as the detail.
    let r = run(vec![tap(0.5, "agent/t3"), hover(1.0, "outline/tick/T1")]);
    assert_eq!(r.find("outline/card/detail").unwrap()["text"], "to do · from b-242");
}

#[test]
fn pressing_a_tick_keeps_its_card_until_pressed_again() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(1.0, "outline/tick/T1"), hover(1.3, "grid")]);
    assert_eq!(r.find("outline/card/title").unwrap()["text"], "Renew the certificate");
    let r = run(vec![tap(0.5, "agent/t3"), tap(1.0, "outline/tick/T1"), tap(1.3, "outline/tick/T1"), hover(1.6, "grid")]);
    assert!(!r.has("outline/card"));
}

/// ⌥⇥ with the terminal in front.
fn enter(t: f64) -> Value {
    json!({ "t": t, "type": "key", "name": "Tab", "alt": true })
}

#[test]
fn option_tab_reaches_the_first_tick_and_plain_tab_stays_the_shells() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), enter(1.0)]);
    assert_eq!(r.find("outline/tick/T1").unwrap()["focused"], true, "{:?}", r.find("outline/tick/T1"));
    assert_eq!(r.find("outline/tick/T1").unwrap()["focusVisible"], true, "the kit's ring shows");
    assert_eq!(r.find("outline/tick/T2").unwrap()["focused"], false);
    assert_eq!(r.data("t3"), "", "the shell got nothing");
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), key(1.0, "Tab")]);
    assert_eq!(r.data("t3"), "\t");
    assert_eq!(r.find("outline/tick/T1").unwrap()["focused"], false);
}

#[test]
fn arrows_move_between_ticks_and_enter_opens_one() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), enter(1.0), key(1.2, "Down")]);
    assert_eq!(r.find("outline/tick/T2").unwrap()["focused"], true);
    assert_eq!(r.data("t3"), "", "arrows are the outline's, not the shell's");
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), enter(1.0), key(1.2, "Down"), key(1.4, "Enter")]);
    assert_eq!(r.find("outline/card/title").unwrap()["text"], "CI failed on #329");
    assert_eq!(r.data("t3"), "");
}

#[test]
fn escape_gives_focus_back_and_the_next_key_reaches_the_shell() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), enter(1.0), key(1.2, "Escape"), key(1.4, "Down"), typed(1.6, "ls")]);
    assert_eq!(r.find("outline/tick/T1").unwrap()["focused"], false);
    assert_eq!(r.data("t3"), "\x1b[Bls", "Esc was the outline's; the arrow and text reached the shell");
}

#[test]
fn a_clicked_tick_does_not_keep_focus_but_enter_does() {
    let r = run(vec![tap(0.5, "agent/t3"), tap(1.0, "outline/tick/T1"), key(1.2, "Down"), typed(1.4, "ls")]);
    assert_eq!(r.find("outline/card/title").unwrap()["text"], "Renew the certificate", "the click opened it");
    assert_eq!(r.find("outline/tick/T2").unwrap()["focused"], false);
    assert_eq!(r.data("t3"), "\x1b[Bls", "the arrow and text reached the shell");
    // ⏎ on a focused tick keeps focus: ↓ still moves to the next.
    let r = run(vec![tap(0.5, "agent/t3"), tap(0.7, "grid"), enter(1.0), key(1.2, "Enter"), key(1.4, "Down")]);
    assert_eq!(r.find("outline/tick/T2").unwrap()["focused"], true);
}

#[test]
fn a_tick_is_a_button_labelled_with_its_headline() {
    let r = run(vec![tap(0.5, "agent/t3")]);
    let t2 = r.find("outline/tick/T2").unwrap();
    assert_eq!((t2["role"].as_str(), t2["label"].as_str()), (Some("button"), Some("CI failed on #329")));
}
