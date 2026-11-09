//! Selecting an agent shows its terminal (H8), against the in-process fake core (`CONTR_HC_FAKE`, the
//! fixture `tests/fixtures/hc/agents.json` with screens added, log in `CONTR_HC_FAKE_LOG`). The plain
//! terminal t20 is restored as tab `t-1`. Tapping `agent/t3` attaches the orchestrator as tab `t-2`
//! (framed, with history); tapping again selects it. Closing an agent's tab only detaches (a stream
//! CLOSE, no `terminal.kill` or `terminal.rm`); closing a plain tab kills and removes it. No test
//! dials a real hc.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Run {
    tree: Value,
    log: Vec<Value>,
    dir: PathBuf,
}

fn tap(t: f64, key: &str) -> Value { json!({ "t": t, "type": "tap", "key": key }) }

/// ⌘ + `name` on macOS, Ctrl+Shift + `name` elsewhere.
fn chord(t: f64, name: &str) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac })
}

fn run(events: Vec<Value>) -> Run {
    let mut events = events;
    events.push(json!({ "t": 8.0, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir().join(format!("contr-rune-hcagenttab-{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("contr");
    std::fs::create_dir_all(&proj).unwrap();
    let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
    for t in fx["terminals"].as_array_mut().unwrap() {
        t["cwd"] = json!(proj);
    }
    fx["screens"] = json!({"t3": "orchestrator-screen\r\n", "t17": "builder-screen\r\n", "t20": "plain-screen\r\n"});
    let (script, out, log, fixture) = (dir.join("script.json"), dir.join("tree.json"), dir.join("log.jsonl"), dir.join("fixture.json"));
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    std::fs::write(&fixture, fx.to_string()).unwrap();
    let r = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env_remove("CONTR_HC")
        .env("CONTR_HC_FAKE", &fixture)
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", &proj)
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
    fn elements(&self) -> &Vec<Value> { self.tree["elements"].as_array().unwrap() }

    fn find(&self, key: &str) -> Option<&Value> { self.elements().iter().find(|e| e["key"] == key) }

    fn tabs(&self) -> Vec<String> {
        self.elements().iter().filter(|e| e["role"] == "tab").map(|e| e["key"].as_str().unwrap().to_owned()).collect()
    }

    /// The key of the selected tab.
    fn active(&self) -> String {
        let found: Vec<&Value> = self.elements().iter().filter(|e| e["role"] == "tab" && e["itemSelected"] == true).collect();
        assert_eq!(found.len(), 1, "one selected tab: {found:?}");
        found[0]["key"].as_str().unwrap().to_owned()
    }

    fn agent_selected(&self, address: &str) -> bool {
        self.find(&format!("agent/{address}")).unwrap_or_else(|| panic!("no agent/{address}"))["itemSelected"] == true
    }

    fn calls(&self, method: &str) -> Vec<&Value> { self.log.iter().filter(|l| l["method"] == method).collect() }

    fn attaches(&self, terminal: &str) -> Vec<&Value> {
        self.log.iter().filter(|l| l["method"] == "terminal.attach" && l["params"]["terminal"] == terminal).map(|l| &l["params"]).collect()
    }

    fn closes(&self, terminal: &str) -> usize {
        self.log.iter().filter(|l| l["frame"] == "close" && l["terminal"] == terminal).count()
    }

    fn grid(&self) -> String {
        self.elements()
            .iter()
            .filter(|e| e["key"].as_str().is_some_and(|k| k.strip_prefix('l').is_some_and(|n| n.parse::<u32>().is_ok())))
            .filter_map(|e| e["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[test]
fn tapping_an_agent_opens_its_terminal_as_a_tab_and_shows_its_screen() {
    let r = run(vec![tap(3.0, "agent/t3")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "the restored plain tab, then the agent's: {:?}", r.log);
    assert_eq!(r.active(), "t-2");
    assert!(r.grid().contains("orchestrator-screen"), "{:?}", r.grid());
    assert!(r.agent_selected("t3"), "the sidebar follows");
    let a = r.attaches("t3");
    assert_eq!(a.len(), 1, "{:?}", r.log);
    assert_eq!(a[0]["history"], 1000);
    assert!(r.calls("terminal.run").is_empty(), "an agent's terminal is attached, never run: {:?}", r.log);
}

#[test]
fn the_tab_shows_the_agent_title_and_state_glyph_not_the_prompt_icon() {
    let r = run(vec![tap(3.0, "agent/t3")]);
    let tab = r.find("t-2").unwrap();
    assert!(tab.to_string().contains("Plan H1"), "the agent's title labels the tab: {tab}");
    let plain = r.find("t-1").unwrap();
    assert!(!plain.to_string().contains("Plan H1"));
    // The glyph (key `glyph`, label = state word) is in the tab as well as in the sidebar row.
    let glyphs: Vec<&Value> = r.elements().iter().filter(|e| e["key"] == "glyph" && !e["path"].as_str().unwrap_or("").contains("/agent/")).collect();
    assert_eq!(glyphs.len(), 1, "one glyph outside the sidebar rows, in the agent's tab: {glyphs:?}");
    assert_eq!(glyphs[0]["label"], "waiting", "the orchestrator state, as in its row");
    assert!(glyphs[0]["path"].as_str().unwrap().contains("t-2"), "{}", glyphs[0]);
}

#[test]
fn tapping_the_same_agent_again_selects_the_tab_instead_of_opening_another() {
    let r = run(vec![tap(3.0, "agent/t3"), tap(4.0, "t-1"), tap(5.0, "agent/t3")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "no second tab");
    assert_eq!(r.attaches("t3").len(), 1);
    assert_eq!(r.active(), "t-2");
    assert!(r.agent_selected("t3"));
}

#[test]
fn the_tab_strip_and_the_sidebar_follow_the_selection_both_ways() {
    let r = run(vec![tap(3.0, "agent/t3"), tap(4.0, "t-1")]);
    assert_eq!(r.active(), "t-1");
    assert!(!r.agent_selected("t3"), "a plain tab clears the agent selection");
    assert!(r.grid().contains("plain-screen"), "{:?}", r.grid());
    let r = run(vec![tap(3.0, "agent/t3"), tap(4.0, "agent/t17")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2", "t-3"]);
    assert_eq!(r.active(), "t-3");
    assert!(r.agent_selected("t17") && !r.agent_selected("t3"), "one agent at a time");
    assert!(r.grid().contains("builder-screen"), "{:?}", r.grid());
    let r = run(vec![tap(3.0, "agent/t3"), tap(4.0, "agent/t17"), tap(5.0, "t-2")]);
    assert!(r.agent_selected("t3") && !r.agent_selected("t17"), "selecting an agent's tab selects its row");
}

#[test]
fn closing_an_agent_tab_only_detaches() {
    let r = run(vec![tap(3.0, "agent/t3"), chord(4.0, "w")]);
    assert_eq!(r.tabs(), vec!["t-1"], "the tab is gone");
    assert_eq!(r.closes("t3"), 1, "a stream CLOSE: {:?}", r.log);
    assert!(r.calls("terminal.kill").is_empty() && r.calls("terminal.rm").is_empty(), "the agent lives on: {:?}", r.log);
    assert!(!r.agent_selected("t3"));
}

#[test]
fn closing_a_plain_tab_still_kills_and_removes_it() {
    let r = run(vec![chord(3.0, "w")]);
    assert!(r.tabs().is_empty());
    let kills = r.calls("terminal.kill");
    assert_eq!(kills.len(), 1, "{:?}", r.log);
    assert_eq!(kills[0]["params"]["terminal"], "t20");
    assert_eq!(r.calls("terminal.rm").len(), 1);
}
