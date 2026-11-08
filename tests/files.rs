//! The files panel (K6): the selected project's tree, read from a temp repo with `--tree` and
//! driven with `--script`. Keys: `files`, `files/head`, `files/title`, `files-changed-only`, `tree`
//! and one row per path (`src/lib.rs`), each with `label`, `added` and `removed` children.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("contr-rune-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Fixture { root: root.canonicalize().unwrap() }
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// A repo with one commit, then changes. Committed: `src/lib.rs` (3 lines), `src/util/mod.rs`,
    /// `docs/readme.md`, `gone/old.txt` (2 lines), `z.txt`, `.gitignore`. After: `src/lib.rs` gets +3
    /// -1, `gone/old.txt` is deleted (-2), `src/util/new.rs` (4 lines) is untracked, and
    /// `ignored.txt` and `build/out.o` are ignored. Expect +7 -3 and 3 files.
    fn make_repo(&self) -> PathBuf {
        let repo = self.dir("repo");
        for d in ["src/util", "docs", "gone", "build"] {
            std::fs::create_dir_all(repo.join(d)).unwrap();
        }
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C").arg(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        let w = |p: &str, c: &str| std::fs::write(repo.join(p), c).unwrap();
        w("src/lib.rs", "a\nb\nc\n");
        w("src/util/mod.rs", "m\n");
        w("docs/readme.md", "r\n");
        w("gone/old.txt", "x\ny\n");
        w("z.txt", "z\n");
        w(".gitignore", "ignored.txt\nbuild/\n");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        w("src/lib.rs", "a\nB\nc\nd\ne\n");
        std::fs::remove_file(repo.join("gone/old.txt")).unwrap();
        w("src/util/new.rs", "1\n2\n3\n4\n");
        w("ignored.txt", "no\n");
        w("build/out.o", "no\n");
        repo
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run(project: &Path, events: Vec<Value>) -> Value {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("contr-rune-files-run-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let (script, out) = (dir.join("script.json"), dir.join("tree.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", project)
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs"])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    v
}

fn elements(tree: &Value) -> &Vec<Value> {
    tree["elements"].as_array().unwrap()
}

fn find<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    elements(tree).iter().find(|e| e["key"] == key)
}

fn rect(tree: &Value, key: &str) -> Vec<f64> {
    find(tree, key).unwrap_or_else(|| panic!("no {key}"))["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

/// The tree rows (role `treeitem`), top to bottom, as their paths.
fn rows(tree: &Value) -> Vec<String> {
    let mut rows: Vec<(f64, String)> = elements(tree)
        .iter()
        .filter(|e| e["role"] == "treeitem")
        .map(|e| (e["rect"][1].as_f64().unwrap(), e["key"].as_str().unwrap().to_owned()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|r| r.1).collect()
}

/// The text of `child` inside the row `row`.
fn row_text(tree: &Value, row: &str, child: &str) -> Option<String> {
    let suffix = format!("/{row}/changes/{child}");
    elements(tree)
        .iter()
        .find(|e| e["key"] == child && e["path"].as_str().is_some_and(|p| p.ends_with(&suffix)))
        .and_then(|e| e["text"].as_str().map(str::to_owned))
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn summary_text(tree: &Value) -> String {
    let e = find(tree, "diff-summary").expect("diff summary");
    let own = format!("{}/", e["path"].as_str().unwrap());
    elements(tree).iter().filter(|c| c["path"].as_str().is_some_and(|p| p.starts_with(&own))).filter_map(|c| c["text"].as_str()).collect()
}

#[test]
fn lists_the_rows_in_order_without_ignored_files() {
    let fx = Fixture::new("order");
    let t = run(&fx.make_repo(), vec![]);
    // Folders first; `docs` has no change so it starts closed; `gone` and `src` hold changes so they start open.
    assert_eq!(
        rows(&t),
        ["docs", "gone", "gone/old.txt", "src", "src/util", "src/util/mod.rs", "src/util/new.rs", "src/lib.rs", ".gitignore", "z.txt"]
    );
    assert!(find(&t, "ignored.txt").is_none() && find(&t, "build").is_none() && find(&t, "build/out.o").is_none());
    assert!(find(&t, "docs/readme.md").is_none(), "a closed folder hides its files");
}

#[test]
fn change_counts_sit_on_the_changed_rows_and_match_the_summary() {
    let fx = Fixture::new("counts");
    let t = run(&fx.make_repo(), vec![]);
    assert_eq!(row_text(&t, "src/lib.rs", "added").as_deref(), Some("+3"));
    assert_eq!(row_text(&t, "src/lib.rs", "removed").as_deref(), Some("\u{2212}1"));
    assert_eq!(row_text(&t, "src/util/new.rs", "added").as_deref(), Some("+4"));
    assert_eq!(row_text(&t, "gone/old.txt", "removed").as_deref(), Some("\u{2212}2"));
    assert_eq!(row_text(&t, "z.txt", "added"), None);
    assert_eq!(row_text(&t, "src/util/mod.rs", "added"), None);
    // The title bar sums the same list: +3+4, -1-2, three files.
    assert_eq!(summary_text(&t), "+7  \u{2212}3  3 files");
}

#[test]
fn changed_only_keeps_changed_files_and_their_folders() {
    let fx = Fixture::new("only");
    let t = run(&fx.make_repo(), vec![tap(0.2, "files-changed-only")]);
    assert_eq!(rows(&t), ["gone", "gone/old.txt", "src", "src/util", "src/util/new.rs", "src/lib.rs"]);
    let off = run(&fx.dir("repo"), vec![tap(0.2, "files-changed-only"), tap(0.4, "files-changed-only")]);
    assert_eq!(rows(&off).len(), 10, "pressing again shows everything");
    let rest = run(&fx.dir("repo"), vec![]);
    assert_ne!(find(&t, "files-changed-only").unwrap()["fill"], find(&rest, "files-changed-only").unwrap()["fill"], "pressed fill differs");
}

#[test]
fn folder_click_toggles_it() {
    let fx = Fixture::new("toggle");
    let repo = fx.make_repo();
    let opened = run(&repo, vec![tap(0.2, "docs")]);
    assert_eq!(rows(&opened)[..2], ["docs", "docs/readme.md"]);
    let closed = run(&repo, vec![tap(0.2, "src")]);
    assert_eq!(rows(&closed), ["docs", "gone", "gone/old.txt", "src", ".gitignore", "z.txt"]);
}

#[test]
fn a_file_click_selects_it_without_changing_the_rows() {
    let fx = Fixture::new("select");
    let repo = fx.make_repo();
    let none = run(&repo, vec![]);
    let t = run(&repo, vec![tap(0.2, "z.txt")]);
    assert_eq!(rows(&t).len(), 10);
    assert_ne!(find(&t, "z.txt").unwrap()["fill"], find(&none, "z.txt").unwrap()["fill"], "the row is highlighted");
    assert_eq!(find(&t, ".gitignore").unwrap()["fill"], find(&none, ".gitignore").unwrap()["fill"], "the others are not");
}

#[test]
fn head_and_rows_sit_where_the_kit_preview_has_them() {
    let fx = Fixture::new("bounds");
    let t = run(&fx.make_repo(), vec![]);
    // As `kit_preview --tree neutral`: head 28 high at the panel's 10 px padding, the toggle 28 x 28 at its right
    // edge, then the tree and 26 px rows over the panel's inner width.
    assert_eq!(rect(&t, "files/head"), [1410.0, 48.0, 282.0, 28.0]);
    assert_eq!(rect(&t, "files-changed-only"), [1662.0, 48.0, 28.0, 28.0]);
    assert_eq!(rect(&t, "docs")[..], [1410.0, 76.0, 282.0, 26.0]);
    assert_eq!(rect(&t, "gone")[1], 102.0);
    assert_eq!(rect(&t, "tree")[..3], [1410.0, 76.0, 282.0]);
    let title = find(&t, "files/title").unwrap();
    assert_eq!(title["text"], "repo");
}

#[test]
fn a_folder_that_is_not_a_repo_lists_its_files_and_shows_no_changes() {
    let fx = Fixture::new("plain");
    let dir = fx.dir("plain");
    for d in ["a/b", "target", "node_modules/x", ".git"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    for f in ["a/b/c.txt", "top.txt", "target/t", "node_modules/x/i.js", ".git/HEAD"] {
        std::fs::write(dir.join(f), "x\n").unwrap();
    }
    let t = run(&dir, vec![]);
    assert_eq!(rows(&t), ["a", "top.txt"]);
    assert!(find(&t, "diff-summary").is_none());
}

#[test]
fn changed_only_in_a_clean_folder_says_so() {
    let fx = Fixture::new("clean");
    let dir = fx.dir("plain");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "x\n").unwrap();
    let t = run(&dir, vec![tap(0.2, "files-changed-only")]);
    assert!(rows(&t).is_empty());
    assert!(elements(&t).iter().any(|e| e["text"] == "No changed files"));
}
