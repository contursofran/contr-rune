//! Saved projects (C5): a project added from the folder dialog is written to the projects file
//! (`CONTR_PROJECTS_FILE`), and a launch without `CONTR_PROJECTS` reads it back, selected.

use serde_json::json;

mod common;
use common::*;

#[test]
fn an_added_project_comes_back_selected_at_the_next_launch() {
    let root = std::env::temp_dir().join(format!("contr-rune-projects-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let root = {
        std::fs::create_dir_all(root.join("first")).unwrap();
        std::fs::create_dir_all(root.join("added")).unwrap();
        root.canonicalize().unwrap()
    };
    let file = root.join("projects.json");
    let launch = |events| {
        App::new()
            .env("CONTR_PROJECTS", "")
            .env("CONTR_PROJECTS_FILE", file.to_string_lossy())
            .events(events)
            .end(1.0)
            .run()
    };

    // A file with one folder; add a second one through the dialog.
    std::fs::write(&file, json!({ "version": 1, "open": [root.join("first")] }).to_string()).unwrap();
    let first = launch(vec![tap(0.3, "add-project"), json!({ "t": 0.5, "type": "file-dialog", "path": root.join("added") })]);
    assert!(first.has("project-1"), "the folder is added");

    let again = launch(vec![]);
    assert!(again.has("project-1"), "the added project is back");
    assert_eq!(again.text_in("project-1", "label").as_deref(), Some("added"));
    // The launch opens a terminal in the selected project; its row sits right under that project's row.
    let keys: Vec<&str> = again.elements().iter().filter_map(|e| e["key"].as_str()).collect();
    let at = |k: &str| keys.iter().position(|x| *x == k).unwrap_or_else(|| panic!("no {k}"));
    assert!(again.selected("term-1"));
    assert!(at("project-1") < at("term-1") && !keys.contains(&"project-2"), "the terminal is in `added`");
    assert!(!again.has("project-2"));
    let _ = std::fs::remove_dir_all(&root);
}
