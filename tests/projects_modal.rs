//! The projects modal (C6): "Open projects…" in the Projects `+` menu opens it; the filter narrows
//! the open list; `close` hides a project (gone from the sidebar, kept in the projects file) and
//! `open` in `recent` brings it back; dragging a grip reorders; esc closes it.

use serde_json::{Value, json};

mod common;
use common::*;

const FILE: &str = "projects.json";

fn run(events: Vec<Value>) -> Run {
    // Three projects: `proj` (with the terminal the launch opens), `alpha` and `beta`.
    App::new().projects(&["alpha", "beta"]).file("CONTR_PROJECTS_FILE", FILE, "").events(events).end(1.5).run()
}

fn open_modal() -> Vec<Value> {
    vec![tap(0.3, "add-project"), tap(0.4, "add-project/menu-item2")]
}

fn saved(t: &Run) -> Value {
    serde_json::from_str(t.file(FILE)).expect("the projects file")
}

/// The last folder names of the paths in `v`.
fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|p| p.as_str().unwrap().rsplit('/').next().unwrap().to_owned()).collect()
}

#[test]
fn the_menu_opens_it_the_filter_narrows_the_list_and_esc_closes_it() {
    let t = run(open_modal());
    assert!(t.has("projects-modal") && t.has("projects-modal/filter") && t.has("projects-modal/done"));
    assert!((0..3).all(|i| t.has(&format!("projects-modal/open/{i}"))), "all three are listed");
    assert!(t.has("projects-modal/grip/1"));

    let mut events = open_modal();
    events.push(typed(0.6, "alp"));
    let t = run(events);
    assert!(t.has("projects-modal/open/1"), "alpha matches");
    assert!(!t.has("projects-modal/open/0") && !t.has("projects-modal/open/2"), "proj and beta do not");
    assert!(!t.has("projects-modal/grip/1"), "no drag under a filter");

    for (name, close) in [("esc key", key(0.6, "Escape")), ("done", tap(0.6, "projects-modal/done")), ("esc chip", tap(0.6, "projects-modal/esc"))] {
        let mut events = open_modal();
        events.push(close);
        let t = run(events);
        assert!(!t.has("projects-modal") && !t.has("projects-modal/scrim"), "{name} closes it");
    }
}

#[test]
fn close_hides_a_project_and_recent_open_brings_it_back() {
    let mut events = open_modal();
    events.push(tap(0.6, "projects-modal/close/1"));
    let t = run(events.clone());
    assert!(!t.has("project-1"), "alpha is gone from the sidebar");
    assert!(t.has("project-0") && t.has("project-2"), "the others keep their numbers");
    assert!(!t.has("projects-modal/open/1") && t.has("projects-modal/recent/0"), "it moved to recent");
    let file = saved(&t);
    assert_eq!((names(&file["open"]), names(&file["hidden"])), (vec!["proj".to_owned(), "beta".to_owned()], vec!["alpha".to_owned()]));
    assert_eq!(file["recent"].as_array().unwrap().len(), 1);
    assert!(file["recent"][0]["at"].as_str().unwrap().ends_with('Z'));

    events.push(tap(0.9, "projects-modal/recent/0/open"));
    let t = run(events);
    assert!(t.has("project-1") && t.selected("project-1"), "alpha is back, selected");
    assert!(!t.has("projects-modal/recent/0"), "and is no longer in recent");
    assert!(saved(&t)["hidden"].as_array().unwrap().is_empty());
}

#[test]
fn a_dragged_grip_reorders_the_sidebar_and_empty_projects_sit_last() {
    let mut events = open_modal();
    // `proj` (place 0) one row down: after `alpha`.
    events.push(json!({ "t": 0.6, "type": "drag", "key": "projects-modal/grip/0", "dx": 0.0, "dy": 52.0, "dur": 0.3 }));
    let t = run(events);
    assert_eq!(names(&saved(&t)["open"]), ["alpha", "proj", "beta"]);
    assert_eq!(t.text_in("project-0", "label").as_deref(), Some("alpha"), "the projects are renumbered");
    // The terminal followed `proj`, now number 1. D174: it has a terminal, so it sits above the empty `alpha`.
    let keys: Vec<&str> = t.elements().iter().filter_map(|e| e["key"].as_str()).collect();
    let at = |k: &str| keys.iter().position(|x| *x == k).unwrap_or_else(|| panic!("no {k}"));
    assert!(at("project-1") < at("term-1") && at("term-1") < at("project-0") && at("project-0") < at("project-2"));
}
