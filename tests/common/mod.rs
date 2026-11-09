//! Shared helpers for the headless tests: run the app with a script, read back the tree and the
//! fake core's log, and build script events. Use with `mod common; use common::*;`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

/// A scratch folder that is removed when it drops. The counter keeps the names apart inside one
/// test process.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(format!("contr-rune-test-{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj")).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One headless launch: the scene, the fake core's fixture (a file of `tests/fixtures/hc/`; none
/// means the local PTY), the script events and the script time at which the tree is read.
pub struct App {
    scene: Option<&'static str>,
    fixture: Option<&'static str>,
    events: Vec<Value>,
    end: f64,
}

impl App {
    pub fn new() -> App {
        App { scene: None, fixture: None, events: Vec::new(), end: 4.0 }
    }

    pub fn scene(mut self, scene: &'static str) -> App {
        self.scene = Some(scene);
        self
    }

    pub fn fixture(mut self, fixture: &'static str) -> App {
        self.fixture = Some(fixture);
        self
    }

    pub fn events(mut self, events: Vec<Value>) -> App {
        self.events = events;
        self
    }

    pub fn end(mut self, end: f64) -> App {
        self.end = end;
        self
    }

    pub fn run(mut self) -> Run {
        self.events.push(json!({ "t": self.end, "type": "mark", "name": "end" }));
        let scratch = Scratch::new();
        let dir = &scratch.0;
        let (script, out, log) = (dir.join("script.json"), dir.join("tree.json"), dir.join("log.jsonl"));
        std::fs::write(&script, json!({ "events": self.events }).to_string()).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
        cmd.env_remove("CONTR_HC_FAKE")
            .env_remove("CONTR_HC")
            .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
            .env("CONTR_PROJECTS", dir.join("proj"))
            .env("CONTR_HC_FAKE_LOG", &log)
            .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--at", "mark:end"]);
        if let Some(f) = self.fixture {
            cmd.env("CONTR_HC_FAKE", Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc").join(f));
        }
        if let Some(s) = self.scene {
            cmd.args(["--scene", s]);
        }
        let r = cmd.output().expect("run the app");
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let tree = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        let log = std::fs::read_to_string(&log).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
        Run { tree, log }
    }
}

/// The tree at the end of a launch, and the fake core's log.
pub struct Run {
    pub tree: Value,
    pub log: Vec<Value>,
}

impl Run {
    pub fn find(&self, key: &str) -> Option<&Value> {
        self.tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key)
    }

    pub fn rect(&self, key: &str) -> Vec<f64> {
        let e = self.find(key).unwrap_or_else(|| panic!("no element {key}"));
        e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    }

    /// Every text on the page (grid rows included).
    pub fn texts(&self) -> Vec<String> {
        self.tree["elements"].as_array().unwrap().iter().filter_map(|e| e["text"].as_str().map(str::to_owned)).collect()
    }

    /// The rows of one pane's grid as text: `prefix` is "l" for the left pane, "p1/l" for the right one.
    pub fn grid_rows(&self, prefix: &str) -> String {
        self.tree["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["key"].as_str().is_some_and(|k| k.strip_prefix(prefix).is_some_and(|n| n.parse::<u32>().is_ok())))
            .filter_map(|e| e["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The bytes sent as DATA to `terminal`, in order.
    pub fn data(&self, terminal: &str) -> String {
        self.log.iter().filter(|l| l["frame"] == "data" && l["terminal"] == terminal).map(|l| l["bytes"].as_str().unwrap()).collect()
    }

    /// The RESIZE frames sent for `terminal` as (cols, rows), in order.
    pub fn resizes(&self, terminal: &str) -> Vec<(u64, u64)> {
        self.log.iter().filter(|l| l["frame"] == "resize" && l["terminal"] == terminal).map(|l| (l["cols"].as_u64().unwrap(), l["rows"].as_u64().unwrap())).collect()
    }
}

pub fn tap(t: f64, key: &str) -> Value { json!({ "t": t, "type": "tap", "key": key }) }
pub fn typed(t: f64, text: &str) -> Value { json!({ "t": t, "type": "type", "text": text }) }
pub fn key(t: f64, name: &str) -> Value { json!({ "t": t, "type": "key", "name": name }) }
pub fn shift_key(t: f64, name: &str) -> Value { json!({ "t": t, "type": "key", "name": name, "shift": true }) }
pub fn drag(t: f64, key: &str, dx: f64) -> Value { json!({ "t": t, "type": "drag", "key": key, "dx": dx, "dur": 0.3 }) }

/// ⌘ + `name` on macOS, Ctrl+Shift + `name` elsewhere; `back` adds ⇧ on macOS (Ctrl+Alt+Shift elsewhere).
pub fn chord(t: f64, name: &str, back: bool) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac || back, "alt": !mac && back })
}
