//! The title bar's right side (K5): the diff summary of the selected project, the split and search
//! buttons, and the files toggle (button and the shortcut). Keys: `diff-summary`, `split`, `search`,
//! `files-toggle`; panels: `terminal`, `files`.

use std::sync::OnceLock;

use serde_json::Value;

mod common;
use common::*;

fn run(events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").events(events).end(1.5).run()
}

/// A repo with one commit (`a.txt` 3 lines, `bin.dat` binary, `gone.txt` 2 lines), then: `a.txt`
/// gets 3 lines added and 1 removed, `bin.dat` changes, `gone.txt` is deleted (-2), `new.txt`
/// (4 lines) is untracked and `ignored.txt` is ignored. Expect +7 -3 and 4 files.
fn dirty_repo(d: &Dirs) {
    let r = &d.proj;
    git_repo(r, &[("a.txt", "one\ntwo\nthree\n"), ("bin.dat", "\0\u{1}\u{2}\u{3}"), ("gone.txt", "x\ny\n"), (".gitignore", "ignored.txt\n")]);
    write(r, "a.txt", "one\nTWO\nthree\nfour\nfive\n");
    write(r, "bin.dat", [0u8, 9, 9, 9, 9]);
    std::fs::remove_file(format!("{r}/gone.txt")).unwrap();
    write(r, "new.txt", "1\n2\n3\n4\n");
    write(r, "ignored.txt", "no\n");
}

fn repo_run(events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").setup(dirty_repo).events(events).end(1.0).run()
}

/// The launch with the files panel open (the start state), and the one with it hidden by the toggle;
/// shared by the toggle and the pressed-state tests.
fn shown() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(vec![]))
}

fn hidden() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(vec![tap(0.2, "files-toggle")]))
}

/// All the text under the element keyed `key`.
fn text_under(t: &Run, key: &str) -> String {
    let path = t.find(key).unwrap_or_else(|| panic!("no {key}"))["path"].as_str().unwrap();
    let own = format!("{path}/");
    t.elements().iter().filter(|c| c["path"].as_str().is_some_and(|p| p.starts_with(&own))).filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("")
}

#[test]
fn summary_counts_the_working_tree_against_head() {
    let t = repo_run(vec![]);
    // a.txt +3 -1, bin.dat (binary, no lines), gone.txt -2, untracked new.txt +4; ignored.txt not counted.
    assert_eq!(text_under(&t, "diff-summary"), "+7  \u{2212}3  4 files");
}

#[test]
fn summary_is_hidden_for_a_folder_that_is_not_a_repo() {
    let t = shown();
    assert!(!t.has("diff-summary"));
    // The buttons stay.
    for key in ["split", "search", "files-toggle"] {
        assert!(t.has(key), "{key}");
    }
}

#[test]
fn summary_is_hidden_when_the_repo_is_clean() {
    let t = App::new()
        .scene("three-tabs")
        .setup(|d| {
            dirty_repo(d);
            git(&d.proj, &["add", "-A"]);
            git(&d.proj, &["commit", "-q", "-m", "all"]);
        })
        .end(1.0)
        .run();
    assert!(!t.has("diff-summary"));
}

#[test]
fn summary_click_shows_the_files_panel() {
    let t = repo_run(vec![tap(0.2, "files-toggle"), tap(0.4, "diff-summary")]);
    assert!(t.has("files"));
}

#[test]
fn trailing_buttons_sit_in_the_title_bar() {
    let t = repo_run(vec![]);
    // 28 px icon buttons, 8 px apart, ending 8 px from the right edge (as `kit_preview --tree`).
    assert_eq!(t.rect("files-toggle")[2..], [28.0, 28.0]);
    assert_eq!(t.rect("files-toggle")[0], 1710.0 - 8.0 - 28.0);
    assert_eq!(t.rect("search")[0], t.rect("files-toggle")[0] - 8.0 - 28.0);
    assert_eq!(t.rect("split")[0], t.rect("search")[0] - 8.0 - 28.0);
    for key in ["diff-summary", "split", "search", "files-toggle"] {
        let r = t.rect(key);
        assert!(r[1] >= 0.0 && r[1] + r[3] <= 38.0, "{key} inside the 38 px bar");
    }
    assert!(t.rect("diff-summary")[0] + t.rect("diff-summary")[2] <= t.rect("split")[0]);
}

#[test]
fn files_toggle_hides_and_shows_the_files_panel() {
    assert!(shown().has("files"));
    assert_eq!(shown().rect("terminal"), [350.0, 38.0, 1042.0, 1066.0]);

    assert!(!hidden().has("files"));
    // The terminal card takes the freed 302 + 8 px.
    assert_eq!(hidden().rect("terminal"), [350.0, 38.0, 1352.0, 1066.0]);

    let back = run(vec![tap(0.2, "files-toggle"), tap(0.4, "files-toggle")]);
    assert_eq!(back.rect("terminal"), shown().rect("terminal"));
    assert!(back.has("files"));
}

#[test]
fn the_shortcut_toggles_the_files_panel_too() {
    let hidden = run(vec![chord(0.2, "b", false)]);
    assert!(!hidden.has("files"));
    assert_eq!(hidden.rect("terminal")[2], 1352.0);
    let back = run(vec![chord(0.2, "b", false), chord(0.4, "b", false)]);
    assert!(back.has("files"));
    assert_eq!(back.rect("terminal")[2], 1042.0);
}

#[test]
fn the_toggle_shows_its_pressed_state_and_the_other_buttons_have_help() {
    let (on, off) = (shown(), hidden());
    assert_ne!(on.find("files-toggle").unwrap()["fill"], off.find("files-toggle").unwrap()["fill"], "pressed fill differs");
    let help = |t: &Run, key: &str| t.find(key).unwrap()["help"].as_str().unwrap().to_owned();
    // Each tooltip carries its shortcut after the text ("Split terminal  ⌘D").
    assert!(help(on, "split").starts_with("Split terminal"));
    assert!(help(on, "search").starts_with("Search"));
    assert!(help(on, "files-toggle").starts_with("Hide files"));
    assert!(help(off, "files-toggle").starts_with("Show files"));
}
