//! Tabs are hc terminals (H3), against the in-process fake core (`CONTR_HC_FAKE`, with the log in
//! `CONTR_HC_FAKE_LOG`). The new-tab chord and the first tab run a terminal with the Contract's
//! labels, the grid shows the fake's first paint, typed keys go out as DATA and the echo shows in
//! the grid, a split pane's resize goes out as RESIZE, closing a tab sends kill then rm, a CLOSE from
//! the core closes the tab, and a core that cannot run a terminal leaves a local PTY. Elements:
//! tabs `t-<id>`, grid rows `l<N>`. No test dials a real hc.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Run {
    tree: Value,
    log: Vec<Value>,
    project: String,
    dir: PathBuf,
}

/// Run the app headless on a scratch project folder, with `events`, until t = 8 (the terminals are
/// up by t = 2). `fixture` is a file of `tests/fixtures/hc/`, or `None` for no fake core.
fn run(fixture: Option<&str>, scene: Option<&str>, mut events: Vec<Value>) -> Run {
    events.push(json!({ "t": 8.0, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir().join(format!("contr-rune-hctabs-{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let (script, out, log) = (dir.join("script.json"), dir.join("tree.json"), dir.join("log.jsonl"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
    cmd.env_remove("CONTR_HC_FAKE")
        .env_remove("CONTR_HC")
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", &project)
        .env("CONTR_HC_FAKE_LOG", &log)
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--at", "mark:end"]);
    if let Some(f) = fixture {
        cmd.env("CONTR_HC_FAKE", Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc").join(f));
    }
    if let Some(s) = scene {
        cmd.args(["--scene", s]);
    }
    let r = cmd.output().expect("run the app");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let tree = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let log = std::fs::read_to_string(&log).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    Run { tree, log, project: project.to_string_lossy().into_owned(), dir }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Run {
    fn calls(&self, method: &str) -> Vec<&Value> {
        self.log.iter().filter(|l| l["method"] == method).collect()
    }

    /// The bytes sent as DATA to `terminal`, in order.
    fn data(&self, terminal: &str) -> String {
        self.log.iter().filter(|l| l["frame"] == "data" && l["terminal"] == terminal).map(|l| l["bytes"].as_str().unwrap()).collect()
    }

    fn resizes(&self, terminal: &str) -> Vec<(u64, u64)> {
        self.log.iter().filter(|l| l["frame"] == "resize" && l["terminal"] == terminal).map(|l| (l["cols"].as_u64().unwrap(), l["rows"].as_u64().unwrap())).collect()
    }

    fn tabs(&self) -> Vec<String> {
        self.tree["elements"].as_array().unwrap().iter().filter(|e| e["role"] == "tab").map(|e| e["key"].as_str().unwrap().to_owned()).collect()
    }

    /// The rows of the (left) grid as text.
    fn grid(&self) -> String {
        self.tree["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["key"].as_str().is_some_and(|k| k.strip_prefix('l').is_some_and(|n| n.parse::<u32>().is_ok())))
            .filter_map(|e| e["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn tap(t: f64, key: &str) -> Value { json!({ "t": t, "type": "tap", "key": key }) }
fn typed(t: f64, text: &str) -> Value { json!({ "t": t, "type": "type", "text": text }) }
fn enter(t: f64) -> Value { json!({ "t": t, "type": "key", "name": "Enter" }) }

/// ⌘ + `name` on macOS, Ctrl+Shift + `name` elsewhere.
fn chord(t: f64, name: &str) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac })
}

#[test]
fn the_first_tab_runs_a_terminal_with_the_contract_params() {
    let r = run(Some("tabs.json"), None, vec![]);
    let runs = r.calls("terminal.run");
    assert_eq!(runs.len(), 1, "{:?}", r.log);
    let p = &runs[0]["params"];
    assert_eq!(p["argv"], json!([]), "empty argv: the core starts the login shell");
    assert_eq!(p["cwd"], r.project.as_str());
    assert_eq!(p["labels"], json!({"contr": "agent", "role": "terminal", "project": "proj", "worktree": r.project}));
    assert_eq!((&p["no_parent"], &p["outlive_parent"], &p["wait"]), (&json!(true), &json!(true), &json!(false)));
    assert_eq!(p["env"], json!({"TERM": "xterm-256color", "COLORTERM": "truecolor"}));
    assert!(p["size"]["cols"].as_u64().unwrap() > 0 && p["size"]["rows"].as_u64().unwrap() > 0);
    let name = p["name"].as_str().unwrap();
    assert!(name.starts_with("term-") && name[5..].chars().all(|c| c.is_ascii_digit()), "name {name}");
    assert_eq!(r.tabs(), vec!["t-1"]);
}

#[test]
fn the_grid_shows_the_first_paint() {
    let r = run(Some("tabs.json"), None, vec![]);
    assert!(r.grid().starts_with('$'), "first paint of the fake's new terminal: {:?}", r.grid());
}

#[test]
fn the_chord_and_the_plus_button_run_more_terminals() {
    let r = run(Some("tabs.json"), None, vec![chord(2.0, "t"), tap(3.0, "new-tab")]);
    assert_eq!(r.calls("terminal.run").len(), 3);
    assert_eq!(r.tabs(), vec!["t-1", "t-2", "t-3"]);
    let names: Vec<&str> = r.calls("terminal.run").iter().map(|c| c["params"]["name"].as_str().unwrap()).collect();
    assert!(names[0] != names[1] && names[1] != names[2], "{names:?}");
}

#[test]
fn typed_keys_go_out_as_data_and_the_echo_shows_in_the_grid() {
    let r = run(Some("tabs.json"), None, vec![typed(3.0, "ls"), enter(3.5)]);
    assert_eq!(r.data("t1"), "ls\r");
    assert!(r.grid().contains("$ ls"), "echo: {:?}", r.grid());
}

#[test]
fn a_split_pane_resize_goes_out_as_resize() {
    let r = run(Some("tabs.json"), None, vec![tap(3.0, "split")]);
    assert_eq!(r.calls("terminal.run").len(), 2, "the split opens a second terminal");
    let left = r.resizes("t1");
    assert!(left.len() >= 2, "layout, then the split: {left:?}");
    let (before, after) = (left[left.len() - 2].0, left[left.len() - 1].0);
    assert!(after * 10 < before * 6, "the left pane is about half as wide: {before} then {after}");
    // The new pane's terminal hears its own size too (it was created with the full-panel size).
    let right = r.resizes("t2");
    assert!(right.last().is_some_and(|(c, _)| *c * 10 < before * 6), "{right:?}");
}

#[test]
fn closing_a_tab_sends_kill_then_rm() {
    let r = run(Some("tabs.json"), None, vec![chord(2.0, "t"), chord(3.0, "w")]);
    assert_eq!(r.tabs(), vec!["t-1"]);
    let order: Vec<(&str, &str)> = r.log.iter().filter_map(|l| Some((l["method"].as_str()?, l["params"]["terminal"].as_str().unwrap_or("")))).filter(|(m, _)| *m != "terminal.run" && *m != "terminal.attach").collect();
    assert_eq!(order, vec![("terminal.kill", "t2"), ("terminal.rm", "t2")]);
}

#[test]
fn a_close_from_the_core_closes_the_tab() {
    // The fake ends the terminal like a shell that exits when it reads `exit\r`.
    let r = run(Some("tabs.json"), None, vec![chord(2.0, "t"), typed(3.0, "exit"), enter(3.5)]);
    assert_eq!(r.tabs(), vec!["t-1"], "{:?}", r.log);
    assert_eq!(r.data("t2"), "exit\r");
    // The tab's terminal is removed from the core, not left finished.
    assert_eq!(r.calls("terminal.rm").iter().map(|c| c["params"]["terminal"].as_str().unwrap()).collect::<Vec<_>>(), vec!["t2"]);
}

#[test]
fn a_core_that_cannot_run_a_terminal_leaves_a_local_pty() {
    let r = run(Some("tabs-fail.json"), None, vec![typed(4.0, "echo $((6*7))"), enter(4.5)]);
    assert_eq!(r.calls("terminal.run").len(), 1, "tried once");
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty());
    assert_eq!(r.tabs(), vec!["t-1"], "the tab stays");
    assert!(r.grid().lines().any(|l| l.trim() == "42"), "a local shell answered: {:?}", r.grid());
}

#[test]
fn without_the_fake_a_headless_run_uses_local_ptys() {
    let r = run(None, Some("three-tabs"), vec![]);
    assert!(r.log.is_empty(), "no hc call: {:?}", r.log);
    assert_eq!(r.tabs().len(), 3);
}

#[test]
fn a_scene_opens_hc_tabs_too() {
    let r = run(Some("tabs-paint.json"), Some("three-tabs"), vec![]);
    assert_eq!(r.calls("terminal.run").len(), 3);
    assert_eq!(r.tabs().len(), 3);
    assert!(r.grid().contains("Cargo.toml"), "the fixture's first paint: {:?}", r.grid());
}
