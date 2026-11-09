//! Agent rows in the sidebar (H5), against the in-process fake core (`CONTR_HC_FAKE`, fixture
//! `tests/fixtures/hc/agents.json`: an orchestrator, two workers and a plain terminal). Each test
//! patches the fixture into a scratch folder: the project folder `contr` is real and the ages are
//! set relative to now. Under a project: the orchestrator, its workers one level deeper, then the
//! terminal tabs. Keys: `agent/<address>` (with `glyph`, `subtitle`, `detail`, `label` inside),
//! `project-<i>`, `term-<id>`. Bounds equal the `kit_preview --tree` agent rows. No test dials a
//! real hc.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const MIN: u64 = 60_000;

struct Run {
    tree: Value,
    dir: PathBuf,
}

struct Dirs {
    proj: String,
    other: String,
    gone: String,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// The shared fixture with the project folder and ages filled in: t3 is 14 min old, t17 6 min,
/// t18 2 min, the terminal t20 4 min.
fn base(d: &Dirs) -> Value {
    let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
    let now = now_ms();
    for (addr, ago) in [("t3", 14), ("t17", 6), ("t18", 2), ("t20", 4)] {
        let t = fx["terminals"].as_array_mut().unwrap().iter_mut().find(|t| t["address"] == addr).unwrap();
        t["cwd"] = json!(d.proj);
        t["status_ms"] = json!(now - ago * MIN);
    }
    fx
}

fn agent(address: &str, parent: Option<&str>, role: &str, project: &str, cwd: &str, ago_min: u64) -> Value {
    json!({
        "address": address, "name": format!("n-{address}"), "parent": parent, "cwd": cwd, "argv": ["claude", "--model", "sonnet"],
        "labels": {"contr": "agent", "role": role, "project": project}, "state": "live", "title": format!("Title {address}"),
        "created_ms": 5000 + address.trim_start_matches('t').parse::<u64>().unwrap(), "status_ms": now_ms() - ago_min * MIN,
        "status": {"state": "idle", "msg": ""}
    })
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}

fn event(after_ms: u64, seq: u64, ty: &str, terminal: &str, data: Value) -> Value {
    json!({"after_ms": after_ms, "method": "event", "params": {"seq": seq, "type": ty, "terminal": terminal, "data": data}})
}

/// Run the app headless until t = 6 (the first listing is in at once, the fixture's events by 300 ms).
fn run(fx: impl Fn(&Dirs) -> Value, mut events: Vec<Value>) -> Run {
    events.push(json!({ "t": 6.0, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir().join(format!("contr-rune-hcagents-{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let dirs = Dirs { proj: dir.join("contr").to_string_lossy().into_owned(), other: dir.join("other").to_string_lossy().into_owned(), gone: dir.join("gone").to_string_lossy().into_owned() };
    std::fs::create_dir_all(&dirs.proj).unwrap();
    std::fs::create_dir_all(&dirs.other).unwrap();
    let (script, out, fixture) = (dir.join("script.json"), dir.join("tree.json"), dir.join("fixture.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    std::fs::write(&fixture, fx(&dirs).to_string()).unwrap();
    let r = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env_remove("CONTR_HC")
        .env("CONTR_HC_FAKE", &fixture)
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", &dirs.proj)
        .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "agents", "--at", "mark:end"])
        .output()
        .expect("run the app");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    Run { tree: serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap(), dir }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Run {
    fn elements(&self) -> &Vec<Value> {
        self.tree["elements"].as_array().unwrap()
    }

    fn find(&self, key: &str) -> Option<&Value> {
        self.elements().iter().find(|e| e["key"] == key)
    }

    fn has(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    fn rect(&self, key: &str) -> Vec<f64> {
        let e = self.find(key).unwrap_or_else(|| panic!("no {key}"));
        e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    }

    /// The element keyed `child` inside the row keyed `row`.
    fn child(&self, row: &str, child: &str) -> Option<&Value> {
        let marker = format!("/{row}/");
        self.elements().iter().find(|e| e["key"] == child && e["path"].as_str().is_some_and(|p| p.contains(&marker)))
    }

    /// The text of the element keyed `child` inside the row keyed `row`.
    fn text(&self, row: &str, child: &str) -> Option<String> {
        self.child(row, child).and_then(|e| e["text"].as_str().map(str::to_owned))
    }

    /// The agent rows (`agent/<address>`), top to bottom.
    fn agents(&self) -> Vec<String> {
        let mut rows: Vec<(f64, String)> = self
            .elements()
            .iter()
            .filter_map(|e| e["key"].as_str().filter(|k| k.starts_with("agent/")).map(|k| (e["rect"][1].as_f64().unwrap(), k.trim_start_matches("agent/").to_owned())))
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|r| r.1).collect()
    }

    fn selected(&self, key: &str) -> bool {
        self.find(key).unwrap_or_else(|| panic!("no {key}"))["itemSelected"] == true
    }
}

#[test]
fn agents_nest_under_their_project_with_the_kit_bounds_and_texts() {
    let r = run(base, vec![]);
    assert_eq!(r.agents(), vec!["t3", "t17", "t18"], "the orchestrator, then its workers in order");
    // Same numbers as `kit_preview --tree`: project row 30, orchestrator 48 at depth 1, workers 44 at depth 2.
    assert_eq!(r.rect("project-0"), [18.0, 84.0, 314.0, 30.0]);
    assert_eq!(r.rect("agent/t3"), [18.0, 116.0, 314.0, 48.0]);
    assert_eq!(r.rect("agent/t17"), [18.0, 166.0, 314.0, 44.0]);
    assert_eq!(r.rect("agent/t18"), [18.0, 212.0, 314.0, 44.0]);
    // The plain terminal tab stays a terminal row, after the agents.
    assert_eq!(r.rect("term-1"), [18.0, 258.0, 314.0, 48.0]);
    assert!(!r.has("agent/t20"), "a role=terminal terminal is a tab, not an agent");
    // Titles, meta lines and ages.
    assert_eq!(r.text("agent/t3", "label").as_deref(), Some("Plan H1"));
    assert_eq!(r.text("agent/t17", "label").as_deref(), Some("Build H1"));
    assert_eq!(r.text("agent/t3", "subtitle").as_deref(), Some("opus"));
    assert_eq!(r.text("agent/t17", "subtitle").as_deref(), Some("b-241  sonnet"));
    assert_eq!(r.text("agent/t18", "subtitle").as_deref(), Some("b-242  sonnet"));
    assert_eq!(r.text("agent/t3", "detail").as_deref(), Some("14m"));
    assert_eq!(r.text("agent/t17", "detail").as_deref(), Some("6m"));
    assert_eq!(r.text("agent/t18", "detail").as_deref(), Some("2m"));
    // The glyph column: the Claude mark (no text) for the orchestrator, the working glyph for t17.
    assert!(r.child("agent/t3", "glyph").is_some_and(|g| g["text"].is_null()));
    assert_eq!(r.text("agent/t17", "glyph").as_deref(), Some("⠋"));
    assert!(r.child("agent/t18", "glyph").is_some());
    // The workers sit one level (16) in from the orchestrator's mark.
    let x = |row: &str| r.child(row, "avatar").unwrap()["rect"][0].as_f64().unwrap();
    assert_eq!(x("agent/t17") - x("agent/t3"), 16.0);
}

#[test]
fn a_created_event_adds_a_row_and_a_removed_one_drops_it() {
    let r = run(
        |d| {
            let mut fx = base(d);
            fx["events"] = json!([
                event(100, 1, "created", "t30", agent("t30", Some("t3"), "builder", "contr", &d.proj, 0)),
                event(250, 2, "removed", "t17", json!({})),
            ]);
            fx
        },
        vec![],
    );
    assert_eq!(r.agents(), vec!["t3", "t18", "t30"], "t30 came, t17 went");
    assert_eq!(r.text("agent/t30", "label").as_deref(), Some("Title t30"));
    assert_eq!(r.text("agent/t30", "subtitle").as_deref(), Some("n-t30  sonnet"));
    assert_eq!(r.text("agent/t30", "detail").as_deref(), Some("now"));
    assert_eq!(r.text("agent/t30", "glyph").as_deref(), Some("⠶"), "an idle worker");
    assert_eq!(r.rect("agent/t30")[3], 44.0, "a worker row");
}

#[test]
fn a_finished_agent_leaves_the_sidebar() {
    let r = run(
        |d| {
            let mut fx = base(d);
            fx["events"] = json!([event(100, 1, "finished", "t18", json!({"reason": "exited", "exit_code": 0, "signal": null, "finished_ms": 1}))]);
            fx
        },
        vec![],
    );
    assert_eq!(r.agents(), vec!["t3", "t17"]);
}

#[test]
fn a_title_event_renames_the_row() {
    let r = run(
        |d| {
            let mut fx = base(d);
            fx["events"] = json!([event(100, 1, "title", "t17", json!({"title": "Build H1 again"}))]);
            fx
        },
        vec![],
    );
    assert_eq!(r.text("agent/t17", "label").as_deref(), Some("Build H1 again"));
}

#[test]
fn a_click_selects_one_agent_and_a_terminal_row_takes_it_back() {
    let r = run(base, vec![tap(2.0, "agent/t17")]);
    assert!(r.selected("agent/t17"));
    assert!(!r.selected("agent/t3") && !r.selected("agent/t18"));
    let r = run(base, vec![tap(2.0, "agent/t17"), tap(2.5, "agent/t18")]);
    assert!(r.selected("agent/t18") && !r.selected("agent/t17"), "one at a time");
    let r = run(base, vec![tap(2.0, "agent/t17"), tap(2.5, "term-1")]);
    assert!(!r.selected("agent/t17") && r.selected("term-1"));
    let r = run(base, vec![]);
    assert!(!r.selected("agent/t3") && !r.selected("agent/t17"), "no agent is selected at the start");
}

#[test]
fn an_agent_of_a_closed_project_opens_it_and_a_missing_folder_is_skipped() {
    let r = run(
        |d| {
            let mut fx = base(d);
            let list = fx["terminals"].as_array_mut().unwrap();
            list.push(agent("t40", None, "orchestrator", "other", &d.other, 3));
            list.push(agent("t41", Some("t40"), "builder", "other", &d.other, 1));
            list.push(agent("t42", None, "orchestrator", "gone", &d.gone, 1));
            fx
        },
        vec![],
    );
    assert!(r.has("project-1") && !r.has("project-2"), "the folder that exists became a project, the one that is gone did not");
    assert_eq!(r.agents(), vec!["t3", "t17", "t18", "t40", "t41"]);
    assert!(r.rect("agent/t40")[1] > r.rect("project-1")[1], "t40 is under project-1");
    assert!(!r.has("agent/t42"));
}

#[test]
fn a_foreign_claude_terminal_is_an_orchestrator_row() {
    let r = run(
        |d| {
            let mut fx = base(d);
            let bare = |address: &str, name: &str| {
                let mut t = agent(address, None, "x", "contr", &d.proj, 9);
                t["labels"] = json!({});
                t["argv"] = json!([]);
                t["status"] = Value::Null;
                t["foreground"] = json!({"pid": 1, "name": name, "argv": [], "cwd": d.proj});
                t
            };
            fx["terminals"].as_array_mut().unwrap().extend([bare("t50", "2.1.30"), bare("t51", "zsh")]);
            fx
        },
        vec![],
    );
    assert_eq!(r.agents(), vec!["t3", "t17", "t18", "t50"]);
    assert_eq!(r.rect("agent/t50")[3], 48.0, "an orchestrator row");
    assert_eq!(r.text("agent/t50", "subtitle").as_deref(), Some("orchestrator"));
}

#[test]
fn without_hc_there_are_no_agent_rows() {
    let dir = std::env::temp_dir().join(format!("contr-rune-hcagents-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("contr")).unwrap();
    let out = dir.join("tree.json");
    let r = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env_remove("CONTR_HC_FAKE")
        .env("CONTR_HC", "off")
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", dir.join("contr"))
        .args(["--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "agents"])
        .output()
        .expect("run the app");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let tree: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert!(!tree["elements"].as_array().unwrap().iter().any(|e| e["key"].as_str().is_some_and(|k| k.starts_with("agent/"))));
    let _ = std::fs::remove_dir_all(&dir);
}
