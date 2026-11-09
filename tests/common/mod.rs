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

/// The folders of one launch: the project (`CONTR_PROJECTS`), a second one that exists and one
/// that does not.
pub struct Dirs {
    /// The scratch folder that holds all the others.
    pub root: String,
    pub proj: String,
    pub other: String,
    pub gone: String,
}

impl Scratch {
    fn new() -> Scratch {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let id = format!("{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(format!("contr-rune-test-{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("other")).unwrap();
        // The real path (`/var` is `/private/var` on macOS): the app compares project folders by it.
        Scratch(dir.canonicalize().unwrap())
    }

    fn dirs(&self, project: &str) -> Dirs {
        let path = |n: &str| self.0.join(n).to_string_lossy().into_owned();
        Dirs { root: self.0.to_string_lossy().into_owned(), proj: path(project), other: path("other"), gone: path("gone") }
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
    fixture_with: Option<Box<dyn FnOnce(&Dirs) -> Value>>,
    project: &'static str,
    envs: Vec<(&'static str, String)>,
    more_projects: Vec<&'static str>,
    setup: Option<Box<dyn FnOnce(&Dirs)>>,
    files: Vec<(&'static str, &'static str, String)>,
    events: Vec<Value>,
    end: f64,
}

impl App {
    pub fn new() -> App {
        App { scene: None, fixture: None, fixture_with: None, project: "proj", envs: Vec::new(), more_projects: Vec::new(), setup: None, files: Vec::new(), events: Vec::new(), end: 2.0 }
    }

    /// A fixture made in code from the launch's folders (written into the scratch folder).
    pub fn fixture_with(mut self, f: impl FnOnce(&Dirs) -> Value + 'static) -> App {
        self.fixture_with = Some(Box::new(f));
        self
    }

    /// The name of the project folder (the default is `proj`).
    pub fn project(mut self, name: &'static str) -> App {
        self.project = name;
        self
    }

    /// More project folders (names in the scratch folder, made empty) after the first in `CONTR_PROJECTS`.
    pub fn projects(mut self, names: &[&'static str]) -> App {
        self.more_projects = names.to_vec();
        self
    }

    /// Runs on the launch's folders before the app starts (to make a repo in `dirs.proj`, for example).
    pub fn setup(mut self, f: impl FnOnce(&Dirs) + 'static) -> App {
        self.setup = Some(Box::new(f));
        self
    }

    /// One more environment variable for the app.
    pub fn env(mut self, name: &'static str, value: impl Into<String>) -> App {
        self.envs.push((name, value.into()));
        self
    }

    /// A file in the scratch folder with `text` (`{root}` is the scratch folder), its path in the variable `env`; `Run::file(name)` reads it back.
    pub fn file(mut self, env: &'static str, name: &'static str, text: impl Into<String>) -> App {
        self.files.push((env, name, text.into()));
        self
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
        let dirs = scratch.dirs(self.project);
        std::fs::create_dir_all(&dirs.proj).unwrap();
        let mut projects = dirs.proj.clone();
        for name in &self.more_projects {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            projects = format!("{projects}:{}", dir.join(name).display());
        }
        if let Some(f) = self.setup.take() {
            f(&dirs);
        }
        let (script, out, log) = (dir.join("script.json"), dir.join("tree.json"), dir.join("log.jsonl"));
        // `{root}` in an event (a dialog path, say) is the scratch folder.
        std::fs::write(&script, json!({ "events": self.events }).to_string().replace("{root}", &dirs.root)).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_contr-rune"));
        cmd.env_remove("CONTR_HC_FAKE")
            .env_remove("CONTR_HC")
            .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
            .env("CONTR_AGENTS_CONFIG", dir.join("no-agents.json"))
            .env("CONTR_PROJECTS", &projects)
            .env("CONTR_HC_FAKE_LOG", &log)
            .args(["--script", script.to_str().unwrap(), "--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--at", "mark:end"]);
        if let Some(f) = self.fixture {
            cmd.env("CONTR_HC_FAKE", Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hc").join(f));
        }
        if let Some(f) = self.fixture_with.take() {
            let path = dir.join("fixture.json");
            std::fs::write(&path, f(&dirs).to_string()).unwrap();
            cmd.env("CONTR_HC_FAKE", path);
        }
        for (name, value) in &self.envs {
            cmd.env(name, value);
        }
        for (env, name, text) in &self.files {
            std::fs::write(dir.join(name), text.replace("{root}", &dirs.root)).unwrap();
            cmd.env(env, dir.join(name));
        }
        if let Some(s) = self.scene {
            cmd.args(["--scene", s]);
        }
        let r = cmd.output().expect("run the app");
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let tree = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        let log = std::fs::read_to_string(&log).map(|t| t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()).unwrap_or_default();
        let files = self.files.iter().map(|(_, name, _)| (name.to_string(), std::fs::read_to_string(dir.join(name)).unwrap_or_default())).collect();
        Run { tree, log, project: dirs.proj, files }
    }
}

/// The tree at the end of a launch, and the fake core's log.
pub struct Run {
    pub tree: Value,
    pub log: Vec<Value>,
    /// The project folder (it is removed when the launch ends).
    pub project: String,
    files: Vec<(String, String)>,
}

impl Run {
    /// The text of a file made with `App::file`, as the app left it.
    pub fn file(&self, name: &str) -> &str {
        &self.files.iter().find(|f| f.0 == name).unwrap_or_else(|| panic!("no file {name}")).1
    }

    pub fn elements(&self) -> &Vec<Value> {
        self.tree["elements"].as_array().unwrap()
    }

    pub fn find(&self, key: &str) -> Option<&Value> {
        self.elements().iter().find(|e| e["key"] == key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    /// The fake core's logged calls of `method`.
    pub fn calls(&self, method: &str) -> Vec<&Value> {
        self.log.iter().filter(|l| l["method"] == method).collect()
    }

    /// The `terminal.attach` params sent for `terminal`.
    pub fn attaches(&self, terminal: &str) -> Vec<&Value> {
        self.log.iter().filter(|l| l["method"] == "terminal.attach" && l["params"]["terminal"] == terminal).map(|l| &l["params"]).collect()
    }

    /// The focus frames sent for `terminal`, in order.
    pub fn focus(&self, terminal: &str) -> Vec<bool> {
        self.log.iter().filter(|l| l["frame"] == "focus" && l["terminal"] == terminal).map(|l| l["gained"].as_bool().unwrap()).collect()
    }

    /// How many stream CLOSE frames the app sent for `terminal`.
    pub fn closes(&self, terminal: &str) -> usize {
        self.log.iter().filter(|l| l["frame"] == "close" && l["terminal"] == terminal).count()
    }

    /// The keys of the elements with `role`, in tree order.
    pub fn keys(&self, role: &str) -> Vec<String> {
        self.elements().iter().filter(|e| e["role"] == role).map(|e| e["key"].as_str().unwrap().to_owned()).collect()
    }

    /// The tab keys (`t-<id>`), in tree order.
    pub fn tabs(&self) -> Vec<String> {
        self.keys("tab")
    }

    /// The key of the selected tab.
    pub fn active_tab(&self) -> String {
        let found: Vec<&Value> = self.elements().iter().filter(|e| e["role"] == "tab" && e["itemSelected"] == true).collect();
        assert_eq!(found.len(), 1, "one selected tab: {found:?}");
        found[0]["key"].as_str().unwrap().to_owned()
    }

    /// Whether the element keyed `key` is the selected item of its list.
    pub fn selected(&self, key: &str) -> bool {
        self.find(key).unwrap_or_else(|| panic!("no {key}"))["itemSelected"] == true
    }

    /// The element keyed `child` inside the row keyed `row`.
    pub fn child(&self, row: &str, child: &str) -> Option<&Value> {
        let marker = format!("/{row}/");
        self.elements().iter().find(|e| e["key"] == child && e["path"].as_str().is_some_and(|p| p.contains(&marker)))
    }

    /// The text of the element keyed `child` inside the row keyed `row`.
    pub fn text_in(&self, row: &str, child: &str) -> Option<String> {
        self.child(row, child).and_then(|e| e["text"].as_str().map(str::to_owned))
    }

    /// The rect of the element keyed `child` inside the row keyed `row`.
    pub fn rect_in(&self, row: &str, child: &str) -> Vec<f64> {
        let e = self.child(row, child).unwrap_or_else(|| panic!("no {child} in {row}"));
        e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    }

    /// The rows of the left pane's grid as text.
    pub fn grid(&self) -> String {
        self.grid_rows("l")
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

/// A timed event for the fake core's fixture (`events`).
pub fn hc_event(after_ms: u64, seq: u64, ty: &str, terminal: &str, data: Value) -> Value {
    json!({"after_ms": after_ms, "method": "event", "params": {"seq": seq, "type": ty, "terminal": terminal, "data": data}})
}

pub fn hover(t: f64, key: &str) -> Value { json!({ "t": t, "type": "move", "key": key }) }

/// A git command in `repo` (a fixed identity, no signing); panics when it fails.
pub fn git(repo: &str, args: &[&str]) {
    let out = Command::new("git")
        .args(["-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Writes `text` to `rel` inside `repo`, making the folders.
pub fn write(repo: &str, rel: &str, text: impl AsRef<[u8]>) {
    let path = Path::new(repo).join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A git repo in `repo` with one commit of `files`; the test then changes the working tree.
pub fn git_repo(repo: &str, files: &[(&str, &str)]) {
    git(repo, &["init", "-q"]);
    for (rel, text) in files {
        write(repo, rel, text);
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "init"]);
}

/// A folder that only looks like a repo on `branch` (a `.git/HEAD`).
pub fn fake_repo(repo: &str, branch: &str) {
    write(repo, ".git/HEAD", format!("ref: refs/heads/{branch}\n"));
}
