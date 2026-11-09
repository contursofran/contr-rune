//! The title bar's right side (K5): the diff summary of the selected project, the split and search
//! buttons, and the files toggle (button and the shortcut). Keys: `diff-summary`, `split`, `search`,
//! `files-toggle`; panels: `terminal`, `files`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("contr-rune-tb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("plain")).unwrap();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        Fixture { root: root.canonicalize().unwrap() }
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// A repo with one commit (`a.txt` 3 lines, `bin.dat` binary, `gone.txt` 2 lines), then: `a.txt`
    /// gets 2 lines added and 1 removed, `bin.dat` changes, `gone.txt` is deleted (-2), `new.txt`
    /// (4 lines) is untracked and `ignored.txt` is ignored. Expect +7 -3 and 4 files.
    fn make_repo(&self) {
        let repo = self.dir("repo");
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
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(repo.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(repo.join("gone.txt"), "x\ny\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\nfour\nfive\n").unwrap(); // +3 -1 (TWO replaces two)
        std::fs::write(repo.join("bin.dat"), [0u8, 9, 9, 9, 9]).unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        std::fs::write(repo.join("new.txt"), "1\n2\n3\n4\n").unwrap();
        std::fs::write(repo.join("ignored.txt"), "no\n").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run(projects: &[&Path], events: Vec<Value>) -> Value {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("contr-rune-tb-run-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let (script, out) = (dir.join("script.json"), dir.join("tree.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let list: Vec<String> = projects.iter().map(|p| p.display().to_string()).collect();
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", list.join(":"))
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs"])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    v
}

fn find<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key)
}

fn rect(tree: &Value, key: &str) -> Vec<f64> {
    find(tree, key).unwrap_or_else(|| panic!("no {key}"))["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

/// All the text under the element keyed `key`.
fn text_in(tree: &Value, key: &str) -> String {
    let e = find(tree, key).unwrap_or_else(|| panic!("no {key}"));
    let path = e["path"].as_str().unwrap();
    let own = format!("{path}/");
    tree["elements"].as_array().unwrap().iter()
        .filter(|c| c["path"].as_str().is_some_and(|p| p.starts_with(&own)))
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn chord_b(t: f64) -> Value {
    // ⌘B on macOS, Ctrl+Shift+B elsewhere.
    json!({ "t": t, "type": "key", "name": "b", "cmd": true, "shift": !cfg!(target_os = "macos") })
}

#[test]
fn summary_counts_the_working_tree_against_head() {
    let fx = Fixture::new("summary");
    fx.make_repo();
    let t = run(&[&fx.dir("repo")], vec![]);
    // a.txt +3 -1, bin.dat (binary, no lines), gone.txt -2, untracked new.txt +4; ignored.txt not counted.
    let text = text_in(&t, "diff-summary");
    assert_eq!(text, "+7  \u{2212}3  4 files");
}

#[test]
fn summary_is_hidden_for_a_folder_that_is_not_a_repo() {
    let fx = Fixture::new("plain");
    let t = run(&[&fx.dir("plain")], vec![]);
    assert!(find(&t, "diff-summary").is_none());
    // The buttons stay.
    for key in ["split", "search", "files-toggle"] {
        assert!(find(&t, key).is_some(), "{key}");
    }
}

#[test]
fn summary_is_hidden_when_the_repo_is_clean() {
    let fx = Fixture::new("clean");
    fx.make_repo();
    let repo = fx.dir("repo");
    for args in [&["add", "-A"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "all"][..]] {
        assert!(Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success());
    }
    let t = run(&[&repo], vec![]);
    assert!(find(&t, "diff-summary").is_none());
}

#[test]
fn summary_click_shows_the_files_panel() {
    let fx = Fixture::new("click");
    fx.make_repo();
    let t = run(&[&fx.dir("repo")], vec![tap(0.2, "files-toggle"), tap(0.4, "diff-summary")]);
    assert!(find(&t, "files").is_some());
}

#[test]
fn trailing_buttons_sit_in_the_title_bar() {
    let fx = Fixture::new("bounds");
    fx.make_repo();
    let t = run(&[&fx.dir("repo")], vec![]);
    // 28 px icon buttons, 8 px apart, ending 8 px from the right edge (as `kit_preview --tree`).
    assert_eq!(rect(&t, "files-toggle")[2..], [28.0, 28.0]);
    assert_eq!(rect(&t, "files-toggle")[0], 1710.0 - 8.0 - 28.0);
    assert_eq!(rect(&t, "search")[0], rect(&t, "files-toggle")[0] - 8.0 - 28.0);
    assert_eq!(rect(&t, "split")[0], rect(&t, "search")[0] - 8.0 - 28.0);
    for key in ["diff-summary", "split", "search", "files-toggle"] {
        let r = rect(&t, key);
        assert!(r[1] >= 0.0 && r[1] + r[3] <= 38.0, "{key} inside the 38 px bar");
    }
    assert!(rect(&t, "diff-summary")[0] + rect(&t, "diff-summary")[2] <= rect(&t, "split")[0]);
}

#[test]
fn files_toggle_hides_and_shows_the_files_panel() {
    let fx = Fixture::new("toggle");
    let plain = fx.dir("plain");
    let shown = run(&[&plain], vec![]);
    assert!(find(&shown, "files").is_some());
    assert_eq!(rect(&shown, "terminal"), [350.0, 38.0, 1042.0, 1066.0]);

    let hidden = run(&[&plain], vec![tap(0.2, "files-toggle")]);
    assert!(find(&hidden, "files").is_none());
    // The terminal card takes the freed 302 + 8 px.
    assert_eq!(rect(&hidden, "terminal"), [350.0, 38.0, 1352.0, 1066.0]);

    let back = run(&[&plain], vec![tap(0.2, "files-toggle"), tap(0.4, "files-toggle")]);
    assert_eq!(rect(&back, "terminal"), rect(&shown, "terminal"));
    assert!(find(&back, "files").is_some());
}

#[test]
fn the_shortcut_toggles_the_files_panel_too() {
    let fx = Fixture::new("chord");
    let plain = fx.dir("plain");
    let hidden = run(&[&plain], vec![chord_b(0.2)]);
    assert!(find(&hidden, "files").is_none());
    assert_eq!(rect(&hidden, "terminal")[2], 1352.0);
    let back = run(&[&plain], vec![chord_b(0.2), chord_b(0.4)]);
    assert!(find(&back, "files").is_some());
    assert_eq!(rect(&back, "terminal")[2], 1042.0);
}

#[test]
fn the_toggle_shows_its_pressed_state_and_the_other_buttons_have_help() {
    let fx = Fixture::new("help");
    let plain = fx.dir("plain");
    let on = run(&[&plain], vec![]);
    let off = run(&[&plain], vec![tap(0.2, "files-toggle")]);
    assert_ne!(find(&on, "files-toggle").unwrap()["fill"], find(&off, "files-toggle").unwrap()["fill"], "pressed fill differs");
    assert!(find(&on, "split").unwrap()["help"].as_str().unwrap().starts_with("Split terminal"), "carries its shortcut (\"Split terminal  ⌘D\")");
    // Search carries its shortcut too ("Search  ⌘F").
    assert!(find(&on, "search").unwrap()["help"].as_str().unwrap().starts_with("Search"));
    // The tooltip carries the shortcut after the text ("Hide files  ⌘B").
    assert!(find(&on, "files-toggle").unwrap()["help"].as_str().unwrap().starts_with("Hide files"));
    assert!(find(&off, "files-toggle").unwrap()["help"].as_str().unwrap().starts_with("Show files"));
}
