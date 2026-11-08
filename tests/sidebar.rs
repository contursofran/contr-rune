//! The sidebar (K4): `--script` drives clicks and the folder dialog against the `three-tabs`
//! scene and `--tree` reads the result. Keys: `project-<i>`, `add-<i>`, `add-project`,
//! `term-<id>`, `server-this-mac`, `settings`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

/// Two projects: `repo` (a fake repo on branch `feat/x`, three terminals) and `empty`.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("contr-rune-sb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        std::fs::write(root.join("repo/.git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        std::fs::create_dir_all(root.join("long-project-name/.git")).unwrap();
        std::fs::write(root.join("long-project-name/.git/HEAD"), "ref: refs/heads/worktree-agent-a562bb76be1982d77\n").unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::create_dir_all(root.join("added")).unwrap();
        Fixture { root: root.canonicalize().unwrap() }
    }
    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run(fx: &Fixture, events: Vec<Value>) -> Value {
    let projects = format!("{}:{}", fx.dir("repo").display(), fx.dir("empty").display());
    run_with(fx, &projects, events)
}

fn run_with(fx: &Fixture, projects: &str, events: Vec<Value>) -> Value {
    let script = fx.root.join("script.json");
    let out = fx.root.join("tree.json");
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", projects)
        .args([
            "--script", script.to_str().unwrap(),
            "--tree", out.to_str().unwrap(),
            "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs",
        ])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

fn elements(tree: &Value) -> &Vec<Value> {
    tree["elements"].as_array().unwrap()
}

fn find<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    elements(tree).iter().find(|e| e["key"] == key)
}

fn rect(tree: &Value, key: &str) -> Vec<f64> {
    let e = find(tree, key).unwrap_or_else(|| panic!("no {key}"));
    e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

/// The text of the element keyed `child` inside the row keyed `row`.
fn row_text(tree: &Value, row: &str, child: &str) -> Option<String> {
    let marker = format!("/{row}/");
    elements(tree)
        .iter()
        .find(|e| e["key"] == child && e["path"].as_str().is_some_and(|p| p.contains(&marker)))
        .and_then(|e| e["text"].as_str().map(str::to_owned))
}

/// The id of the selected tab.
fn active_tab(tree: &Value) -> String {
    let found: Vec<&Value> = elements(tree).iter().filter(|e| e["role"] == "tab" && e["itemSelected"] == true).collect();
    assert_eq!(found.len(), 1, "one selected tab, got {found:?}");
    found[0]["key"].as_str().unwrap().trim_start_matches("t-").to_owned()
}

fn tab_count(tree: &Value) -> usize {
    elements(tree).iter().filter(|e| e["role"] == "tab" && e["key"].as_str().is_some_and(|k| k.starts_with("t-"))).count()
}

/// The terminal rows (`term-<id>`), in order.
fn term_rows(tree: &Value) -> Vec<String> {
    let mut rows: Vec<(f64, String)> = elements(tree)
        .iter()
        .filter(|e| e["key"].as_str().is_some_and(|k| k.starts_with("term-")) && e["rect"][3].as_f64() == Some(48.0))
        .map(|e| (e["rect"][1].as_f64().unwrap(), e["key"].as_str().unwrap().to_owned()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|r| r.1).collect()
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn hover(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "move", "key": key })
}

fn selected_row(tree: &Value, key: &str) -> bool {
    find(tree, key).unwrap_or_else(|| panic!("no {key}"))["itemSelected"] == true
}

#[test]
fn bounds_match_the_kit_preview() {
    let fx = Fixture::new("bounds");
    let t = run(&fx, vec![]);
    // Same numbers as `kit_preview --tree`: head, project row (30), terminal rows (48, depth 1).
    assert_eq!(rect(&t, "sidebar"), [8.0, 38.0, 334.0, 1066.0]);
    assert_eq!(rect(&t, "add-project"), [302.0, 48.0, 28.0, 28.0]);
    assert_eq!(rect(&t, "project-0"), [18.0, 84.0, 314.0, 30.0]);
    assert_eq!(rect(&t, "term-1"), [18.0, 116.0, 314.0, 48.0]);
    assert_eq!(rect(&t, "term-2"), [18.0, 166.0, 314.0, 48.0]);
    assert_eq!(rect(&t, "term-3"), [18.0, 216.0, 314.0, 48.0]);
    assert_eq!(rect(&t, "project-1"), [18.0, 278.0, 314.0, 30.0]);
    assert_eq!(rect(&t, "server-this-mac")[2..], [314.0, 30.0]);
    assert_eq!(rect(&t, "settings")[2..], [28.0, 28.0]);
    // The terminal row's avatar sits one indent (24) in from the project's icon.
    let avatar = elements(&t).iter().find(|e| e["key"] == "avatar" && e["path"].as_str().unwrap().contains("/term-1/")).unwrap();
    assert_eq!(avatar["rect"][0].as_f64().unwrap(), 42.0);
}

#[test]
fn rows_show_the_project_meta() {
    let fx = Fixture::new("meta");
    let t = run(&fx, vec![]);
    assert_eq!(row_text(&t, "project-0", "label").as_deref(), Some("repo"));
    assert_eq!(row_text(&t, "project-0", "detail").as_deref(), Some("feat/x"), "the branch comes from .git/HEAD");
    assert_eq!(row_text(&t, "project-1", "label").as_deref(), Some("empty"));
    assert_eq!(row_text(&t, "project-1", "detail").as_deref(), Some("nothing open"));
    assert_eq!(row_text(&t, "term-1", "detail").as_deref(), Some("now"));
    assert!(row_text(&t, "term-1", "label").is_some_and(|l| !l.is_empty()));
    assert!(row_text(&t, "term-1", "subtitle").is_some_and(|l| !l.is_empty()));
    assert_eq!(row_text(&t, "server-this-mac", "label").as_deref(), Some("this Mac"));
    assert_eq!(row_text(&t, "server-this-mac", "detail").as_deref(), Some("local"));
}

#[test]
fn a_folder_that_is_not_a_repo_has_no_branch() {
    let fx = Fixture::new("norepo");
    // Select the empty project, open a terminal there: it has terminals but no branch.
    let t = run(&fx, vec![hover(0.3, "project-1"), tap(0.6, "add-1")]);
    assert_eq!(row_text(&t, "project-1", "detail"), None);
}

#[test]
fn clicking_a_terminal_row_selects_it_and_its_tab() {
    let fx = Fixture::new("select");
    let before = run(&fx, vec![]);
    assert_eq!(active_tab(&before), "3");
    assert!(selected_row(&before, "term-3"));
    let t = run(&fx, vec![tap(0.5, "term-1")]);
    assert_eq!(active_tab(&t), "1", "the tab follows the row");
    assert!(selected_row(&t, "term-1"));
    assert!(!selected_row(&t, "term-3"));
}

#[test]
fn the_project_plus_opens_a_terminal_there() {
    let fx = Fixture::new("plus");
    let t = run(&fx, vec![hover(0.3, "project-1"), tap(0.6, "add-1")]);
    // Four terminals now: three in `repo`, one in `empty`, which is selected and shows its tab.
    assert_eq!(tab_count(&t), 1, "the strip shows the selected project's tabs");
    assert_eq!(term_rows(&t).len(), 4);
    assert_eq!(active_tab(&t), "4");
    assert!(selected_row(&t, "term-4"));
}

#[test]
fn clicking_a_project_selects_its_terminal_or_none() {
    let fx = Fixture::new("project");
    let t = run(&fx, vec![tap(0.5, "project-1")]);
    assert!(selected_row(&t, "project-1"), "an empty project is selected itself");
    assert_eq!(tab_count(&t), 0);
    let t = run(&fx, vec![tap(0.5, "project-1"), tap(0.9, "project-0")]);
    assert_eq!(tab_count(&t), 3);
    assert_eq!(active_tab(&t), "3", "back to the project's last tab");
}

#[test]
fn the_head_plus_adds_a_project_from_the_folder_dialog() {
    let fx = Fixture::new("add");
    let added = fx.dir("added");
    let t = run(&fx, vec![tap(0.5, "add-project"), json!({ "t": 0.8, "type": "file-dialog", "path": added.to_str().unwrap() })]);
    assert_eq!(row_text(&t, "project-2", "label").as_deref(), Some("added"));
    assert_eq!(row_text(&t, "project-2", "detail").as_deref(), Some("nothing open"));
    assert!(selected_row(&t, "project-2"), "the new project is selected");
}

#[test]
fn cancelling_the_dialog_adds_nothing() {
    let fx = Fixture::new("cancel");
    let t = run(&fx, vec![tap(0.5, "add-project"), json!({ "t": 0.8, "type": "file-dialog-cancel" })]);
    assert!(find(&t, "project-2").is_none());
}

#[test]
fn adding_an_existing_folder_selects_it_again() {
    let fx = Fixture::new("dup");
    let repo = fx.dir("repo");
    let t = run(&fx, vec![tap(0.5, "add-project"), json!({ "t": 0.8, "type": "file-dialog", "path": repo.to_str().unwrap() })]);
    assert!(find(&t, "project-2").is_none(), "no second row for the same folder");
    assert_eq!(tab_count(&t), 3);
}

#[test]
fn the_settings_gear_is_there() {
    let fx = Fixture::new("gear");
    let t = run(&fx, vec![]);
    assert!(find(&t, "settings").is_some());
}

/// The rect of the element keyed `child` inside the row keyed `row`.
fn row_rect(tree: &Value, row: &str, child: &str) -> Vec<f64> {
    let marker = format!("/{row}/");
    let e = elements(tree)
        .iter()
        .find(|e| e["key"] == child && e["path"].as_str().is_some_and(|p| p.contains(&marker)))
        .unwrap_or_else(|| panic!("no {child} in {row}"));
    e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

#[test]
fn a_terminal_subtitle_is_never_its_title() {
    let fx = Fixture::new("subtitle");
    let t = run(&fx, vec![]);
    for id in 1..=3 {
        let row = format!("term-{id}");
        let title = row_text(&t, &row, "label").unwrap();
        let sub = row_text(&t, &row, "subtitle");
        assert!(sub.as_deref().is_some_and(|s| !s.is_empty()), "{row} has a subtitle");
        assert_ne!(sub, Some(title), "{row}: the subtitle repeats the title");
        assert_eq!(rect(&t, &row)[3], 48.0);
    }
}

#[test]
fn a_long_branch_keeps_the_project_name() {
    let fx = Fixture::new("longbranch");
    let projects = fx.dir("long-project-name").display().to_string();
    let t = run_with(&fx, &projects, vec![]);
    let detail = row_text(&t, "project-0", "detail").unwrap();
    assert!(detail.ends_with('…') && detail.chars().count() <= 14, "the branch is clipped: {detail}");
    let label = row_rect(&t, "project-0", "label");
    let meta = row_rect(&t, "project-0", "detail");
    assert!(label[2] > meta[2], "the name ({}) is wider than the branch ({})", label[2], meta[2]);
    assert_eq!(row_text(&t, "project-0", "label").as_deref(), Some("long-project-name"));
}

#[test]
fn the_meta_sits_at_the_row_edge_when_not_hovered() {
    let fx = Fixture::new("metaedge");
    let t = run(&fx, vec![]);
    let edge = |row: &str| {
        let r = row_rect(&t, row, "detail");
        r[0] + r[2]
    };
    // Same right edge as a row with no trailing control ("this Mac" / "local").
    let target = edge("server-this-mac");
    assert!((edge("project-0") - target).abs() <= 1.0, "meta right edge {} vs {target}", edge("project-0"));
    assert!((edge("project-1") - target).abs() <= 1.0);
}

#[test]
fn hovering_a_project_shows_the_plus_in_the_meta_slot() {
    let fx = Fixture::new("metahover");
    let t = run(&fx, vec![hover(0.3, "project-0")]);
    let add = rect(&t, "add-0");
    let slot = row_rect(&t, "project-0", "meta-slot");
    assert!((add[0] + add[2] - (slot[0] + slot[2])).abs() <= 1.0, "the + sits at the slot's right edge");
    assert_eq!(rect(&t, "project-0")[3], 30.0, "no layout shift");
}
