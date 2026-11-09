//! A question's impacts, refs and source in the card (A6). The project `contr` has one open question,
//! D7, with three options (the second recommended, so it shows first), an impact for each, a source
//! and four refs; its `origin` is `contursofran/contr`. A ref that is a link calls the opener, which
//! appends the URL to `CONTR_OPEN_LOG` instead of opening a browser.

mod common;

use common::*;
use serde_json::{Value, json};

const OWNER: &str = r##"{"questions": [
  {"id": "D7", "project": "contr", "coordinator": "t3", "question": "Ship the notch this week?",
   "options": ["Ship now", "Wait a week", "Split it"], "recommended": 1,
   "impacts": ["fast but risky", "safe", "two PRs"], "source": "meeting 2026-11-08",
   "refs": ["#12", "src/notch.rs", "https://example.com/a", "issue 3"], "asked": "2026-11-08T10:00:00Z"}
], "todos": []}"##;

fn run(origin: bool, events: Vec<Value>) -> Run {
    App::new()
        .project("contr")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
                t["status"]["state"] = json!("idle");
            }
            fx
        })
        .setup(move |d| {
            if origin {
                write(&d.proj, ".git/config", "[remote \"origin\"]\n\turl = git@github.com:contursofran/contr.git\n");
            }
        })
        .file("CONTR_OWNER_FILE", "owner.json", OWNER)
        .file("CONTR_OPEN_LOG", "opened.log", "")
        .events(events)
        .end(3.0)
        .run()
}

#[test]
fn impacts_sit_under_their_options_and_context_opens_on_press() {
    let r = run(true, vec![tap(0.5, "notch")]);
    let rect = |k: &str| r.rect(k);
    // The recommended option shows first and keeps its own impact ("safe", the second one).
    for (i, impact) in ["safe", "fast but risky", "two PRs"].into_iter().enumerate() {
        let key = format!("qcard/impact/D7/{i}");
        assert_eq!(r.find(&key).unwrap()["text"], impact);
        let option = rect(&format!("qcard/option/D7/{i}"));
        let below = rect(&key);
        assert!(below[1] >= option[1] + option[3] - 1.0, "impact {i} sits under its option: {below:?} vs {option:?}");
    }
    assert!(r.has("qcard/context/D7"));
    assert!(!r.has("qcard/source/D7") && !r.has("qcard/ref/D7/0"), "the Context starts collapsed");

    let r = run(true, vec![tap(0.5, "notch"), tap(1.0, "qcard/context/D7")]);
    assert_eq!(r.find("qcard/source/D7").unwrap()["text"], "Source: meeting 2026-11-08");
    for (i, text) in ["#12", "src/notch.rs", "https://example.com/a", "issue 3"].into_iter().enumerate() {
        let e = r.find(&format!("qcard/ref/D7/{i}")).unwrap();
        assert_eq!(e["text"].as_str().or(e["label"].as_str()), Some(text));
    }
}

#[test]
fn a_ref_link_opens_the_browser() {
    let r = run(true, vec![tap(0.5, "notch"), tap(1.0, "qcard/context/D7"), tap(1.5, "qcard/ref/D7/0"), tap(2.0, "qcard/ref/D7/2")]);
    assert_eq!(r.file("opened.log"), "https://github.com/contursofran/contr/issues/12\nhttps://example.com/a\n");
}

#[test]
fn hash_refs_need_an_origin() {
    let r = run(false, vec![tap(0.5, "notch"), tap(1.0, "qcard/context/D7"), tap(1.5, "qcard/ref/D7/2")]);
    assert_eq!(r.file("opened.log"), "https://example.com/a\n");
    assert!(r.find("qcard/ref/D7/0").is_some_and(|e| e["role"] != "button"), "#12 is dim text without an origin");
}
