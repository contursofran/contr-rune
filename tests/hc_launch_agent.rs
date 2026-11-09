//! The agent saved as selected comes back at launch (Go D173), on the fixture `agents.json`: once
//! hc's first list and the restore are in, a live saved agent is selected and attached once (its
//! tab is reused when it has one); one that is gone gives the normal launch.

mod common;

use common::*;
use serde_json::{Value, json};

fn launch(agent: &str) -> Run {
    let file = format!(r#"{{"version":1,"open":["{{root}}/contr"],"selected":"{{root}}/contr","agent":"{agent}"}}"#);
    App::new()
        .project("contr")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx
        })
        .env("CONTR_PROJECTS", "")
        .file("CONTR_PROJECTS_FILE", "projects.json", file)
        .end(2.0)
        .run()
}

#[test]
fn a_saved_worker_is_selected_and_attached_once() {
    let r = launch("t17");
    assert!(r.selected("agent/t17"), "{:?}", r.log);
    assert_eq!(r.attaches("t17").len(), 1, "{:?}", r.log);
    assert_eq!(r.tabs(), vec!["t-1", "t-3", "t-2"], "the worker's tab joins the set");
    assert_eq!(r.active_tab(), "t-3");
    assert!(r.file("projects.json").contains("\"t17\""), "the choice stays saved");
}

#[test]
fn a_saved_claude_session_reuses_its_tab() {
    let r = launch("t3");
    assert!(r.selected("agent/t3"));
    assert_eq!(r.attaches("t3").len(), 1, "{:?}", r.log);
    assert_eq!(r.active_tab(), "t-1");
}

#[test]
fn a_saved_agent_that_is_gone_gives_the_normal_launch() {
    let r = launch("t99");
    assert!(!r.selected("agent/t3") && !r.selected("agent/t17"));
    assert_eq!(r.tabs(), vec!["t-1", "t-2"]);
    assert!(!r.file("projects.json").contains("t99"), "the gone agent is dropped from the file");
}
