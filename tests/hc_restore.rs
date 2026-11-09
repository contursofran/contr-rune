//! Reopen and re-attach hc terminals (H4), against the in-process fake core (`CONTR_HC_FAKE`, with
//! the log in `CONTR_HC_FAKE_LOG`). Each test writes its fixture into a scratch folder, so the
//! terminals' folders are real. At launch the app's live terminals (labels `contr=agent`,
//! `role=terminal`) come back as tabs in creation order, attached with history; a project that is
//! not open is added when its folder exists; a lost line re-attaches (`resumed`) and keeps the
//! tab; `restarted` re-attaches; `finished` and `removed` close the tab; quitting kills nothing.
//! Elements: tabs `t-<id>`, grid rows `l<N>`, `hc-banner`, sidebar `project-<i>`. No test dials a
//! real hc.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Run {
    tree: Value,
    log: Vec<Value>,
    dir: PathBuf,
}

/// The scratch folders of one run.
struct Dirs {
    proj: String,
    other: String,
    gone: String,
}

fn term(address: &str, created: u64, worktree: &str) -> Value {
    json!({
        "address": address, "name": format!("term-{address}"), "cwd": worktree, "state": "live", "created_ms": created,
        "labels": {"contr": "agent", "role": "terminal", "project": worktree.rsplit('/').next().unwrap(), "worktree": worktree}
    })
}

fn tap(t: f64, key: &str) -> Value { json!({ "t": t, "type": "tap", "key": key }) }

/// Run the app headless with the fixture `fx(dirs)` until t = 8 (the terminals are up by t = 2).
fn run(fx: impl Fn(&Dirs) -> Value, mut events: Vec<Value>) -> Run {
    events.push(json!({ "t": 8.0, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir().join(format!("contr-rune-hcrestore-{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let dirs = Dirs { proj: dir.join("contr-rune").to_string_lossy().into_owned(), other: dir.join("other").to_string_lossy().into_owned(), gone: dir.join("gone").to_string_lossy().into_owned() };
    std::fs::create_dir_all(&dirs.proj).unwrap();
    std::fs::create_dir_all(&dirs.other).unwrap();
    let (script, out, log, fixture) = (dir.join("script.json"), dir.join("tree.json"), dir.join("log.jsonl"), dir.join("fixture.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    std::fs::write(&fixture, fx(&dirs).to_string()).unwrap();
    let r = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env_remove("CONTR_HC")
        .env("CONTR_HC_FAKE", &fixture)
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", &dirs.proj)
        .env("CONTR_HC_FAKE_LOG", &log)
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--at", "mark:end"])
        .output()
        .expect("run the app");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let tree = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let log = std::fs::read_to_string(&log).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
    Run { tree, log, dir }
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

    /// The attaches to `terminal`, with their params.
    fn attaches(&self, terminal: &str) -> Vec<&Value> {
        self.log.iter().filter(|l| l["method"] == "terminal.attach" && l["params"]["terminal"] == terminal).map(|l| &l["params"]).collect()
    }

    fn keys(&self, role: &str) -> Vec<String> {
        self.tree["elements"].as_array().unwrap().iter().filter(|e| e["role"] == role).map(|e| e["key"].as_str().unwrap().to_owned()).collect()
    }

    fn tabs(&self) -> Vec<String> {
        self.keys("tab")
    }

    fn has(&self, key: &str) -> bool {
        self.tree["elements"].as_array().unwrap().iter().any(|e| e["key"] == key)
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

    /// The focus frames sent for `terminal`, in order.
    fn focus(&self, terminal: &str) -> Vec<bool> {
        self.log.iter().filter(|l| l["frame"] == "focus" && l["terminal"] == terminal).map(|l| l["gained"].as_bool().unwrap()).collect()
    }
}

/// Two terminals of the project, created out of address order, with their screens.
fn two(d: &Dirs) -> Value {
    json!({
        "terminals": [term("t5", 2000, &d.proj), term("t3", 1000, &d.proj)],
        "screens": {"t3": "first-screen\r\n$ ", "t5": "second-screen\r\n$ "},
        "echo": true
    })
}

#[test]
fn terminals_reopen_as_tabs_in_creation_order_with_their_first_paint() {
    let r = run(two, vec![tap(3.0, "t-2")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "{:?}", r.log);
    assert!(r.calls("terminal.run").is_empty(), "nothing new is made: {:?}", r.log);
    // t-2 is the later one (t5) and is the tab shown after the tap.
    assert!(r.grid().contains("second-screen"), "{:?}", r.grid());
    assert!(!r.grid().contains("first-screen"));
    for t in ["t3", "t5"] {
        let a = r.attaches(t);
        assert_eq!(a.len(), 1, "{t}: {:?}", r.log);
        assert_eq!(a[0]["history"], 1000);
        assert!(a[0].get("resumed").is_none(), "a launch attach is not a resume");
    }
}

#[test]
fn the_sample_fixture_matches_the_project_by_its_label() {
    // `tests/fixtures/hc/restore.json` (also used for the snapshot) names no folder: the project label
    // `contr-rune` finds the project. Created out of address order: t2, t5, t8 are the tabs 1 to 3.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc/restore.json");
    let r = run(|_| serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(), vec![tap(3.0, "t-3")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2", "t-3"]);
    assert!(r.grid().contains("Cargo.toml  src"), "{:?}", r.grid());
    assert!(r.calls("terminal.run").is_empty());
}

#[test]
fn the_first_restored_tab_is_the_one_shown() {
    let r = run(two, vec![]);
    assert!(r.grid().contains("first-screen"), "{:?}", r.grid());
}

#[test]
fn a_tab_from_a_closed_project_adds_it_and_a_missing_folder_is_skipped() {
    let r = run(
        |d| {
            json!({
                "terminals": [term("t1", 1, &d.proj), term("t2", 2, &d.other), term("t3", 3, &d.gone)],
                "screens": {"t2": "from-other\r\n"}
            })
        },
        vec![tap(3.0, "project-1")],
    );
    assert_eq!(r.tabs().len(), 1, "the new project shows its one tab: {:?}", r.tabs());
    assert!(r.has("project-1"), "the folder that exists became a project");
    assert!(!r.has("project-2"), "the folder that is gone did not");
    assert!(r.grid().contains("from-other"), "{:?}", r.grid());
    assert!(r.attaches("t3").is_empty(), "no attach for the missing folder: {:?}", r.log);
    assert!(r.calls("terminal.run").is_empty());
}

#[test]
fn terminals_that_are_not_this_apps_tabs_are_left_alone_and_one_new_tab_opens() {
    let r = run(
        |d| {
            let mut builder = term("t1", 1, &d.proj);
            builder["labels"]["role"] = json!("builder");
            let mut foreign = term("t2", 2, &d.proj);
            foreign["labels"] = json!({});
            let mut done = term("t3", 3, &d.proj);
            done["state"] = json!("finished");
            json!({"terminals": [builder, foreign, done]})
        },
        vec![],
    );
    assert!(r.log.iter().all(|l| l["method"] != "terminal.attach" || l["params"]["terminal"] == "t4"), "{:?}", r.log);
    assert_eq!(r.calls("terminal.run").len(), 1, "no tab to reopen: one new tab");
    assert_eq!(r.tabs(), vec!["t-1"]);
}

#[test]
fn a_dropped_connection_attaches_again_with_resumed_and_keeps_the_tab() {
    let r = run(
        |d| {
            let mut fx = two(d);
            fx["events"] = json!([{"after_ms": 600, "method": "fake", "params": {"drop": "framed"}}]);
            fx
        },
        vec![],
    );
    assert_eq!(r.tabs(), vec!["t-1", "t-2"]);
    for t in ["t3", "t5"] {
        let a = r.attaches(t);
        assert_eq!(a.len(), 2, "{t}: first attach, then the re-attach: {:?}", r.log);
        assert_eq!(a[1]["resumed"], true);
    }
    assert!(!r.has("hc-banner"), "back online: no banner");
    assert!(r.grid().contains("first-screen"), "{:?}", r.grid());
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty(), "a lost line never kills");
}

#[test]
fn while_the_core_refuses_the_tab_stays_and_says_reconnecting() {
    let r = run(
        |d| {
            let mut fx = two(d);
            fx["events"] = json!([{"after_ms": 600, "method": "fake", "params": {"drop": "framed", "refuse_attach_ms": 600000}}]);
            fx
        },
        vec![],
    );
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "the tabs stay");
    assert!(r.has("hc-banner"), "the quiet reconnecting line shows");
    let banner = r.tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == "hc-banner").unwrap();
    assert!(banner["text"].as_str().unwrap().contains("reconnecting"), "{banner}");
    assert!(r.grid().contains("first-screen"), "the screen stays: {:?}", r.grid());
    assert!(r.attaches("t3").len() >= 2, "it kept trying: {:?}", r.log);
}

#[test]
fn a_restarted_event_attaches_again() {
    let r = run(
        |d| {
            let mut fx = two(d);
            fx["events"] = json!([{"after_ms": 600, "method": "event", "params": {"seq": 1, "type": "restarted", "terminal": "t3", "data": term("t3", 1000, &d.proj)}}]);
            fx
        },
        vec![],
    );
    assert_eq!(r.tabs(), vec!["t-1", "t-2"]);
    assert_eq!(r.attaches("t3").len(), 2, "{:?}", r.log);
    assert_eq!(r.attaches("t5").len(), 1, "only the restarted terminal");
    assert!(r.grid().contains("first-screen"), "{:?}", r.grid());
}

#[test]
fn finished_and_removed_events_close_their_tabs() {
    let r = run(
        |d| {
            let mut fx = two(d);
            fx["terminals"].as_array_mut().unwrap().push(term("t7", 3000, &d.proj));
            fx["events"] = json!([
                {"after_ms": 600, "method": "event", "params": {"seq": 1, "type": "finished", "terminal": "t3", "data": {"reason": "exited", "exit_code": 0, "signal": null, "finished_ms": 1}}},
                {"after_ms": 700, "method": "event", "params": {"seq": 2, "type": "removed", "terminal": "t7", "data": {}}}
            ]);
            fx
        },
        vec![],
    );
    // t3 was tab 1 and t7 tab 3: only t5 (tab 2) is left.
    assert_eq!(r.tabs(), vec!["t-2"], "{:?}", r.log);
    assert!(r.grid().contains("second-screen"), "{:?}", r.grid());
}

#[test]
fn quitting_leaves_the_terminals_running() {
    let r = run(two, vec![]);
    assert_eq!(r.tabs().len(), 2);
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty(), "{:?}", r.log);
}

#[test]
fn focus_frames_follow_the_selected_tab() {
    let r = run(two, vec![tap(3.0, "t-2")]);
    assert_eq!(r.focus("t3"), vec![true, false], "{:?}", r.log);
    assert_eq!(r.focus("t5"), vec![true]);
}
