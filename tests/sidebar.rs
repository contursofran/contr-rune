//! The sidebar (K4): `--script` drives clicks and the folder dialog against the `three-tabs`
//! scene and `--tree` reads the result. Keys: `project-<i>`, `add-<i>`, `add-project`,
//! `term-<id>`, `server-this-mac`, `settings`.

use std::sync::OnceLock;

use serde_json::{Value, json};

mod common;
use common::*;

/// Two projects: `repo` (a fake repo on branch `feat/x`, three terminals) and `empty`; a folder `added`
/// for the dialog.
fn app() -> App {
    App::new()
        .scene("three-tabs")
        .project("repo")
        .projects(&["empty"])
        .setup(|d| {
            fake_repo(&d.proj, "feat/x");
            std::fs::create_dir_all(format!("{}/added", d.root)).unwrap();
        })
        .end(1.0)
}

fn run(events: Vec<Value>) -> Run {
    app().events(events).run()
}

/// The launch with no events, shared by the tests that only read it.
fn start() -> &'static Run {
    static START: OnceLock<Run> = OnceLock::new();
    START.get_or_init(|| run(vec![]))
}

/// The terminal rows (`term-<id>`), in order.
fn term_rows(t: &Run) -> Vec<String> {
    let mut rows: Vec<(f64, String)> = t
        .elements()
        .iter()
        .filter(|e| e["key"].as_str().is_some_and(|k| k.starts_with("term-")) && e["rect"][3].as_f64() == Some(48.0))
        .map(|e| (e["rect"][1].as_f64().unwrap(), e["key"].as_str().unwrap().to_owned()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|r| r.1).collect()
}

#[test]
fn bounds_match_the_kit_preview() {
    let t = start();
    // Same numbers as `kit_preview --tree`: head, project row (30), terminal rows (48, depth 1).
    assert_eq!(t.rect("sidebar"), [8.0, 38.0, 334.0, 1066.0]);
    assert_eq!(t.rect("add-project"), [302.0, 48.0, 28.0, 28.0]);
    assert_eq!(t.rect("project-0"), [18.0, 84.0, 314.0, 30.0]);
    assert_eq!(t.rect("term-1"), [18.0, 116.0, 314.0, 48.0]);
    assert_eq!(t.rect("term-2"), [18.0, 166.0, 314.0, 48.0]);
    assert_eq!(t.rect("term-3"), [18.0, 216.0, 314.0, 48.0]);
    assert_eq!(t.rect("project-1"), [18.0, 278.0, 314.0, 30.0]);
    assert_eq!(t.rect("server-this-mac")[2..], [314.0, 30.0]);
    assert_eq!(t.rect("settings")[2..], [28.0, 28.0]);
    // The terminal row's avatar sits one indent (24) in from the project's icon.
    assert_eq!(t.rect_in("term-1", "avatar")[0], 42.0);
}

#[test]
fn rows_show_the_project_meta() {
    let t = start();
    let text = |row: &str, child: &str| t.text_in(row, child);
    assert_eq!(text("project-0", "label").as_deref(), Some("repo"));
    assert_eq!(text("project-0", "detail").as_deref(), Some("feat/x"), "the branch comes from .git/HEAD");
    assert_eq!(text("project-1", "label").as_deref(), Some("empty"));
    assert_eq!(text("project-1", "detail").as_deref(), Some("nothing open"));
    assert_eq!(text("term-1", "detail").as_deref(), Some("now"));
    assert!(text("term-1", "label").is_some_and(|l| !l.is_empty()));
    assert!(text("term-1", "subtitle").is_some_and(|l| !l.is_empty()));
    assert_eq!(text("server-this-mac", "label").as_deref(), Some("this Mac"));
    assert_eq!(text("server-this-mac", "detail").as_deref(), Some("local"));
}

/// Select the empty project and open a terminal there (the `+` shows on hover); shared by the two tests on it.
fn plus_in_empty() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(vec![hover(0.3, "project-1"), tap(0.6, "add-1")]))
}

#[test]
fn a_folder_that_is_not_a_repo_has_no_branch() {
    // It has terminals now but no branch.
    assert_eq!(plus_in_empty().text_in("project-1", "detail"), None);
}

#[test]
fn clicking_a_terminal_row_selects_it_and_its_tab() {
    assert_eq!(start().active_tab(), "t-3");
    assert!(start().selected("term-3"));
    let t = run(vec![tap(0.5, "term-1")]);
    assert_eq!(t.active_tab(), "t-1", "the tab follows the row");
    assert!(t.selected("term-1"));
    assert!(!t.selected("term-3"));
}

#[test]
fn the_project_plus_opens_a_terminal_there() {
    let t = plus_in_empty();
    // Four terminals now: three in `repo`, one in `empty`, which is selected and shows its tab.
    assert_eq!(t.tabs().len(), 1, "the strip shows the selected project's tabs");
    assert_eq!(term_rows(t).len(), 4);
    assert_eq!(t.active_tab(), "t-4");
    assert!(t.selected("term-4"));
}

#[test]
fn clicking_a_project_selects_its_terminal_or_none() {
    let t = run(vec![tap(0.5, "project-1")]);
    assert!(t.selected("project-1"), "an empty project is selected itself");
    assert_eq!(t.tabs().len(), 0);
    let t = run(vec![tap(0.5, "project-1"), tap(0.9, "project-0")]);
    assert_eq!(t.tabs().len(), 3);
    assert_eq!(t.active_tab(), "t-3", "back to the project's last tab");
}

#[test]
fn the_head_plus_menu_adds_a_project_from_the_folder_dialog() {
    let t = run(vec![tap(0.5, "add-project"), tap(0.7, "add-project/menu-item1"), json!({ "t": 0.9, "type": "file-dialog", "path": "{root}/added" })]);
    assert_eq!(t.text_in("project-2", "label").as_deref(), Some("added"));
    assert_eq!(t.text_in("project-2", "detail").as_deref(), Some("nothing open"));
    assert!(t.selected("project-2"), "the new project is selected");
}

#[test]
fn cancelling_the_dialog_adds_nothing() {
    let t = run(vec![tap(0.5, "add-project"), tap(0.7, "add-project/menu-item1"), json!({ "t": 0.9, "type": "file-dialog-cancel" })]);
    assert!(!t.has("project-2"));
}

#[test]
fn adding_an_existing_folder_selects_it_again() {
    let t = run(vec![tap(0.5, "add-project"), tap(0.7, "add-project/menu-item1"), json!({ "t": 0.9, "type": "file-dialog", "path": "{root}/repo" })]);
    assert!(!t.has("project-2"), "no second row for the same folder");
    assert_eq!(t.tabs().len(), 3);
}

#[test]
fn a_long_branch_keeps_the_project_name() {
    // The clipping itself is the unit test `long_text_is_clipped_with_an_ellipsis`; this checks the row's bounds.
    let t = App::new().scene("three-tabs").project("long-project-name").setup(|d| fake_repo(&d.proj, "worktree-agent-a562bb76be1982d77")).end(1.0).run();
    let detail = t.text_in("project-0", "detail").unwrap();
    assert!(detail.ends_with('…') && detail.chars().count() <= 14, "the branch is clipped: {detail}");
    let (label, meta) = (t.rect_in("project-0", "label"), t.rect_in("project-0", "detail"));
    assert!(label[2] > meta[2], "the name ({}) is wider than the branch ({})", label[2], meta[2]);
    assert_eq!(t.text_in("project-0", "label").as_deref(), Some("long-project-name"));
}

#[test]
fn the_meta_sits_at_the_row_edge_when_not_hovered() {
    let t = start();
    let edge = |row: &str| {
        let r = t.rect_in(row, "detail");
        r[0] + r[2]
    };
    // Same right edge as a row with no trailing control ("this Mac" / "local").
    let target = edge("server-this-mac");
    assert!((edge("project-0") - target).abs() <= 1.0, "meta right edge {} vs {target}", edge("project-0"));
    assert!((edge("project-1") - target).abs() <= 1.0);
}

#[test]
fn hovering_a_project_shows_the_plus_in_the_meta_slot() {
    let t = run(vec![hover(0.3, "project-0")]);
    let add = t.rect("add-0");
    let slot = t.rect_in("project-0", "meta-slot");
    assert!((add[0] + add[2] - (slot[0] + slot[2])).abs() <= 1.0, "the + sits at the slot's right edge");
    assert_eq!(t.rect("project-0")[3], 30.0, "no layout shift");
}
