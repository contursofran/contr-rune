//! The files panel (K6): the selected project's tree, read from a temp repo with `--tree` and
//! driven with `--script`. Keys: `files`, `files/head`, `files/title`, `files-changed-only`, `tree`
//! and one row per path (`src/lib.rs`), each with `label`, `added` and `removed` children.

use std::sync::OnceLock;

use serde_json::Value;

mod common;
use common::*;

/// A repo with one commit, then changes. Committed: `src/lib.rs` (3 lines), `src/util/mod.rs`,
/// `docs/readme.md`, `gone/old.txt` (2 lines), `z.txt`, `.gitignore`. After: `src/lib.rs` gets +3
/// -1, `gone/old.txt` is deleted (-2), `src/util/new.rs` (4 lines) is untracked, and
/// `ignored.txt` and `build/out.o` are ignored. Expect +7 -3 and 3 files.
fn dirty_repo(d: &Dirs) {
    let r = &d.proj;
    git_repo(
        r,
        &[("src/lib.rs", "a\nb\nc\n"), ("src/util/mod.rs", "m\n"), ("docs/readme.md", "r\n"), ("gone/old.txt", "x\ny\n"), ("z.txt", "z\n"), (".gitignore", "ignored.txt\nbuild/\n")],
    );
    write(r, "src/lib.rs", "a\nB\nc\nd\ne\n");
    std::fs::remove_file(format!("{r}/gone/old.txt")).unwrap();
    write(r, "src/util/new.rs", "1\n2\n3\n4\n");
    write(r, "ignored.txt", "no\n");
    write(r, "build/out.o", "no\n");
}

fn run(events: Vec<Value>) -> Run {
    App::new().scene("three-tabs").project("repo").setup(dirty_repo).events(events).end(1.0).run()
}

/// The launch with no events, shared by the tests that only read it.
fn start() -> &'static Run {
    static START: OnceLock<Run> = OnceLock::new();
    START.get_or_init(|| run(vec![]))
}

/// The tree rows (role `treeitem`), top to bottom, as their keys.
fn rows(t: &Run) -> Vec<String> {
    let mut rows: Vec<(f64, String)> = t.elements().iter().filter(|e| e["role"] == "treeitem").map(|e| (e["rect"][1].as_f64().unwrap(), e["key"].as_str().unwrap().to_owned())).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|r| r.1).collect()
}

fn summary_text(t: &Run) -> String {
    let own = format!("{}/", t.find("diff-summary").expect("diff summary")["path"].as_str().unwrap());
    t.elements().iter().filter(|c| c["path"].as_str().is_some_and(|p| p.starts_with(&own))).filter_map(|c| c["text"].as_str()).collect()
}

#[test]
fn lists_the_rows_in_order_without_ignored_files() {
    let t = start();
    // Folders first; `docs` has no change so it starts closed; `gone` and `src` hold changes so they start open.
    assert_eq!(rows(t), ["docs", "gone", "gone/old.txt", "src", "src/util", "src/util/mod.rs", "src/util/new.rs", "src/lib.rs", ".gitignore", "z.txt"]);
    assert!(!t.has("ignored.txt") && !t.has("build") && !t.has("build/out.o"));
    assert!(!t.has("docs/readme.md"), "a closed folder hides its files");
}

#[test]
fn change_counts_sit_on_the_changed_rows_and_match_the_summary() {
    let t = start();
    assert_eq!(t.text_in("src/lib.rs", "added").as_deref(), Some("+3"));
    assert_eq!(t.text_in("src/lib.rs", "removed").as_deref(), Some("\u{2212}1"));
    assert_eq!(t.text_in("src/util/new.rs", "added").as_deref(), Some("+4"));
    assert_eq!(t.text_in("gone/old.txt", "removed").as_deref(), Some("\u{2212}2"));
    assert_eq!(t.text_in("z.txt", "added"), None);
    assert_eq!(t.text_in("src/util/mod.rs", "added"), None);
    // The title bar sums the same list: +3+4, -1-2, three files.
    assert_eq!(summary_text(t), "+7  \u{2212}3  3 files");
}

#[test]
fn the_changed_only_toggle_filters_the_rows_and_shows_pressed() {
    let on = run(vec![tap(0.2, "files-changed-only")]);
    assert_eq!(rows(&on), ["gone", "gone/old.txt", "src", "src/util", "src/util/new.rs", "src/lib.rs"]);
    assert_ne!(on.find("files-changed-only").unwrap()["fill"], start().find("files-changed-only").unwrap()["fill"], "pressed fill differs");
}

#[test]
fn folder_click_toggles_it() {
    let opened = run(vec![tap(0.2, "docs")]);
    assert_eq!(rows(&opened)[..2], ["docs", "docs/readme.md"]);
    let closed = run(vec![tap(0.2, "src")]);
    assert_eq!(rows(&closed), ["docs", "gone", "gone/old.txt", "src", ".gitignore", "z.txt"]);
}

#[test]
fn a_file_click_selects_it_without_changing_the_rows() {
    let none = start();
    let t = run(vec![tap(0.2, "z.txt")]);
    assert_eq!(rows(&t).len(), 10);
    assert_ne!(t.find("z.txt").unwrap()["fill"], none.find("z.txt").unwrap()["fill"], "the row is highlighted");
    assert_eq!(t.find(".gitignore").unwrap()["fill"], none.find(".gitignore").unwrap()["fill"], "the others are not");
}

#[test]
fn head_and_rows_sit_where_the_kit_preview_has_them() {
    let t = start();
    // As `kit_preview --tree neutral`: head 28 high at the panel's 10 px padding, the toggle 28 x 28 at its right
    // edge, then the tree and 26 px rows over the panel's inner width.
    assert_eq!(t.rect("files/head"), [1410.0, 48.0, 282.0, 28.0]);
    assert_eq!(t.rect("files-changed-only"), [1662.0, 48.0, 28.0, 28.0]);
    assert_eq!(t.rect("docs")[..], [1410.0, 76.0, 282.0, 26.0]);
    assert_eq!(t.rect("gone")[1], 102.0);
    assert_eq!(t.rect("tree")[..3], [1410.0, 76.0, 282.0]);
    assert_eq!(t.find("files/title").unwrap()["text"], "Files");
}

#[test]
fn a_folder_that_is_not_a_repo_lists_its_files_and_shows_no_changes() {
    let t = App::new()
        .scene("three-tabs")
        .setup(|d| {
            for f in ["a/b/c.txt", "top.txt", "target/t", "node_modules/x/i.js", ".git/HEAD"] {
                write(&d.proj, f, "x\n");
            }
        })
        .end(1.0)
        .run();
    assert_eq!(rows(&t), ["a", "top.txt"]);
    assert!(!t.has("diff-summary"));
}

#[test]
fn changed_only_in_a_clean_folder_says_so() {
    let t = App::new().scene("three-tabs").setup(|d| write(&d.proj, "a.txt", "x\n")).events(vec![tap(0.2, "files-changed-only")]).end(1.0).run();
    assert!(rows(&t).is_empty());
    assert!(t.texts().iter().any(|s| s == "No changed files"));
}
