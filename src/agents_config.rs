//! `agents.json`: the profiles, setup commands and flow of the agents, Go-compatible (PARITY-PLAN
//! K2, go:internal/agents/agents.go and flow.go). A file is read over the defaults; a save keeps
//! the keys this code does not know.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// How to start one kind of agent: the program and its arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, deserialize_with = "null_default")]
    pub argv: Vec<String>,
    /// The kit agent definition the profile starts as; empty means the profile name.
    #[serde(default, skip_serializing_if = "String::is_empty", deserialize_with = "null_default")]
    pub agent: String,
}

/// How the orchestrator takes an issue to a merge (D185). A key missing from the file keeps its
/// default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    /// `never`, `when-needed` or `always`.
    pub plan: String,
    /// `none`, `code` or `every`.
    pub review: String,
    /// Failed fix rounds before a debugger.
    pub debugger_after: i64,
    /// The most workers at once, reviewers included.
    pub workers: i64,
    /// `ask` or `auto`.
    pub merge: String,
}

impl Default for Flow {
    fn default() -> Flow {
        Flow { plan: "when-needed".into(), review: "code".into(), debugger_after: 2, workers: 4, merge: "ask".into() }
    }
}

impl Flow {
    /// An error for a value out of range.
    pub fn validate(&self) -> Result<(), String> {
        fn check(key: &str, v: &str, ok: &[&str]) -> Result<(), String> {
            if ok.contains(&v) { Ok(()) } else { Err(format!("flow {key}: {v:?} is not one of {}", ok.join(", "))) }
        }
        check("plan", &self.plan, &["never", "when-needed", "always"])?;
        check("review", &self.review, &["none", "code", "every"])?;
        check("merge", &self.merge, &["ask", "auto"])?;
        if self.debugger_after < 1 {
            return Err(format!("flow debugger_after: {} is less than 1", self.debugger_after));
        }
        if self.workers < 1 {
            return Err(format!("flow workers: {} is less than 1", self.workers));
        }
        Ok(())
    }

    /// The flow as plain rules, one per line, for the orchestrator.
    pub fn lines(&self) -> Vec<String> {
        let plan = match self.plan.as_str() {
            "never" => "Plan first: never. Spawn builder directly; the builder plans in its PR body.",
            "when-needed" => "Plan first: when needed. Spawn planner first only when the work needs several PRs or the issue leaves the approach open.",
            "always" => "Plan first: always. Spawn planner first for each issue, then builder with the plan.",
            _ => "",
        };
        let review = match self.review.as_str() {
            "none" => "Review: none. Spawn no reviewer; read the diff yourself.",
            "code" => "Review: code PRs. Every PR that changes code gets a reviewer; for a docs-only or config-only PR, read the diff yourself.",
            "every" => "Review: every PR gets a reviewer.",
            _ => "",
        };
        let merge = match self.merge.as_str() {
            "ask" => "Merge: ask the owner each time.",
            "auto" => "Merge: merge yourself when CI and review pass.",
            _ => "",
        };
        vec![
            plan.to_string(),
            review.to_string(),
            format!("Debugger: spawn debugger after {} failed fix rounds.", self.debugger_after),
            format!("Workers at once: at most {}, reviewers included.", self.workers),
            merge.to_string(),
        ]
    }

    /// The flow block: "The flow:" and one line per rule.
    pub fn text(&self) -> String {
        format!("The flow:\n- {}\n", self.lines().join("\n- "))
    }
}

/// The profiles by name, the kit folder, the setup commands by main checkout and the flow.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub profiles: BTreeMap<String, Profile>,
    pub kit: String,
    pub setup: BTreeMap<String, String>,
    pub flow: Flow,
    /// Where the profiles come from, for the "no profile" error: the file read, with
    /// "(not found)" after it when the file is missing.
    pub source: String,
}

/// What a file holds; every key is optional.
#[derive(Deserialize)]
struct File {
    profiles: Option<BTreeMap<String, Profile>>,
    kit: Option<String>,
    setup: Option<BTreeMap<String, String>>,
    flow: Option<Flow>,
}

/// A JSON `null` gives the default, as Go's decoder does.
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

impl Default for Config {
    /// The standard profiles: builder sonnet/medium, planner and reviewer opus/low, debugger
    /// opus/xhigh, all `--permission-mode auto`.
    fn default() -> Config {
        let claude = |model: &str, effort: &str| Profile {
            argv: ["claude", "--model", model, "--effort", effort, "--permission-mode", "auto"].map(String::from).to_vec(),
            agent: String::new(),
        };
        let profiles = [
            ("builder", claude("sonnet", "medium")),
            ("planner", claude("opus", "low")),
            ("reviewer", claude("opus", "low")),
            ("debugger", claude("opus", "xhigh")),
        ];
        Config {
            profiles: profiles.into_iter().map(|(n, p)| (n.to_string(), p)).collect(),
            kit: String::new(),
            setup: BTreeMap::new(),
            flow: Flow::default(),
            source: String::new(),
        }
    }
}

/// The value after `flag` (or `flag=`) in `argv`, or "".
pub fn flag_value(argv: &[String], flag: &str) -> String {
    for (i, a) in argv.iter().enumerate() {
        if a == flag {
            if let Some(v) = argv.get(i + 1) {
                return v.clone();
            }
        }
        if let Some(v) = a.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
            return v.to_string();
        }
    }
    String::new()
}

impl Config {
    /// The model the profile starts (`--model`); "" for an unknown profile or none.
    #[allow(dead_code)] // used by D5 (hook stop)
    pub fn model_of(&self, profile: &str) -> String {
        self.profiles.get(profile).map(|p| flag_value(&p.argv, "--model")).unwrap_or_default()
    }

    /// The effort the profile starts (`--effort`); "" for an unknown profile or none.
    #[allow(dead_code)] // used by D5 (hook stop)
    pub fn effort_of(&self, profile: &str) -> String {
        self.profiles.get(profile).map(|p| flag_value(&p.argv, "--effort")).unwrap_or_default()
    }

    /// The error for a profile this config does not have.
    pub fn no_profile(&self, name: &str) -> String {
        let names = self.profiles.keys().cloned().collect::<Vec<_>>().join(", ");
        if self.source.is_empty() {
            format!("agents: no profile {name:?}; profiles: {names}")
        } else {
            format!("agents: no profile {name:?} in {}; profiles: {names}", self.source)
        }
    }

    /// Reads `path` over the defaults: a profile in the file replaces the default of its name, a
    /// flow key missing from the file keeps its default. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Config, String> {
        let mut cfg = Config::default();
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                cfg.source = format!("{} (not found)", path.display());
                return Ok(cfg);
            }
            Err(e) => return Err(format!("agents: {}: {e}", path.display())),
        };
        cfg.source = path.display().to_string();
        let file: File = serde_json::from_str(&data).map_err(|e| format!("agents: {}: {e}", path.display()))?;
        let flow = file.flow.unwrap_or_default();
        flow.validate().map_err(|e| format!("agents: {}: {e}", path.display()))?;
        for (name, p) in file.profiles.unwrap_or_default() {
            if p.argv.is_empty() {
                return Err(format!("agents: {}: profile {name:?} has no argv", path.display()));
            }
            cfg.profiles.insert(name, p);
        }
        if let Some(kit) = file.kit.filter(|k| !k.is_empty()) {
            cfg.kit = kit;
        }
        cfg.setup = file.setup.unwrap_or_default();
        cfg.flow = flow;
        Ok(cfg)
    }

    /// Writes the config to `path` (tmp file, then rename). Keys of the file that this code does
    /// not know stay as they are. A profile goes to the file only when the file had it or when it
    /// differs from the default, so the file does not freeze today's defaults (D186).
    #[allow(dead_code)] // used by C7/C8 (settings tabs)
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |e: &dyn std::fmt::Display| format!("agents: {}: {e}", path.display());
        let mut raw = match std::fs::read_to_string(path) {
            Ok(d) => match serde_json::from_str::<Value>(&d).map_err(|e| fail(&e))? {
                Value::Object(m) => m,
                _ => return Err(fail(&"not a JSON object")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(fail(&e)),
        };
        self.flow.validate().map_err(|e| format!("agents: {e}"))?;
        let had: Option<Map<String, Value>> = match raw.get("profiles") {
            None | Some(Value::Null) => None,
            Some(Value::Object(m)) => Some(m.clone()),
            Some(_) => return Err(fail(&"profiles: not a JSON object")),
        };
        let defaults = Config::default().profiles;
        let profiles: BTreeMap<&String, &Profile> = self
            .profiles
            .iter()
            .filter(|(name, p)| {
                had.as_ref().is_some_and(|h| h.contains_key(*name)) || defaults.get(*name) != Some(*p)
            })
            .collect();
        let mut put = |key: &str, v: Value, empty: bool| {
            if empty { raw.remove(key); } else { raw.insert(key.to_string(), v); }
        };
        put("profiles", value(&profiles), profiles.is_empty() && had.is_none());
        put("kit", Value::String(self.kit.clone()), self.kit.is_empty());
        put("setup", value(&self.setup), self.setup.is_empty());
        put("flow", value(&self.flow), false);
        let mut out = serde_json::to_string_pretty(&raw).map_err(|e| fail(&e))?;
        out.push('\n');
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, out).map_err(|e| fail(&e))?;
        std::fs::rename(&tmp, path).map_err(|e| fail(&e))
    }
}

#[allow(dead_code)] // used by `save`
fn value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("plain data serialises")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FULL: &str = include_str!("../tests/fixtures/agents/full.json");
    const PARTIAL_FLOW: &str = include_str!("../tests/fixtures/agents/partial-flow.json");
    const BAD_FLOW: &str = include_str!("../tests/fixtures/agents/bad-flow.json");
    const NO_ARGV: &str = include_str!("../tests/fixtures/agents/no-argv.json");

    /// A scratch folder with one `agents.json`, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(content: Option<&str>) -> Scratch {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static RUNS: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir()
                .join(format!("contr-rune-agents-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(c) = content {
                std::fs::write(dir.join("agents.json"), c).unwrap();
            }
            Scratch(dir)
        }
        fn path(&self) -> PathBuf {
            self.0.join("agents.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn load(content: &str) -> Result<Config, String> {
        Config::load(&Scratch::new(Some(content)).path())
    }

    #[test]
    fn defaults_are_the_standard_profiles_and_flow() {
        let c = Config::default();
        let got: Vec<_> = ["builder", "planner", "reviewer", "debugger"]
            .map(|n| (c.model_of(n), c.effort_of(n)))
            .to_vec();
        let want = [("sonnet", "medium"), ("opus", "low"), ("opus", "low"), ("opus", "xhigh")]
            .map(|(m, e)| (m.to_string(), e.to_string()));
        assert_eq!(got, want);
        assert!(c.profiles["builder"].argv.ends_with(&["--permission-mode".into(), "auto".into()]));
        assert_eq!(c.flow, Flow { plan: "when-needed".into(), review: "code".into(), debugger_after: 2, workers: 4, merge: "ask".into() });
        assert_eq!((c.model_of("nobody"), c.effort_of("nobody")), (String::new(), String::new()));
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_says_so() {
        let s = Scratch::new(None);
        let c = Config::load(&s.path()).unwrap();
        assert_eq!(c.profiles, Config::default().profiles);
        assert_eq!(c.source, format!("{} (not found)", s.path().display()));
    }

    #[test]
    fn a_file_profile_replaces_the_default_of_its_name() {
        let c = load(FULL).unwrap();
        // `--model=opus` form, and the effort after it.
        assert_eq!((c.model_of("builder"), c.effort_of("builder")), ("opus".into(), "high".into()));
        // The others stay; the new one carries its kit agent.
        assert_eq!(c.model_of("planner"), "opus");
        assert_eq!(c.profiles["planner-high"].agent, "planner");
        assert_eq!(c.profiles.len(), 5);
        assert_eq!(c.kit, "/work/contr/kit");
        assert_eq!(c.setup["~/code/app"], "npm ci");
        assert_eq!(
            c.flow,
            Flow { plan: "always".into(), review: "every".into(), debugger_after: 3, workers: 6, merge: "auto".into() }
        );
    }

    #[test]
    fn a_flow_key_missing_from_the_file_keeps_its_default() {
        let c = load(PARTIAL_FLOW).unwrap();
        assert_eq!(c.flow, Flow { review: "none".into(), ..Flow::default() });
    }

    #[test]
    fn a_bad_file_is_an_error_that_names_the_file() {
        for (content, want) in [
            (BAD_FLOW, "flow plan: \"sometimes\" is not one of never, when-needed, always"),
            (NO_ARGV, "profile \"builder\" has no argv"),
            ("{", "agents: "),
            (r#"{"flow":{"workers":0}}"#, "flow workers: 0 is less than 1"),
            (r#"{"flow":{"debugger_after":0}}"#, "flow debugger_after: 0 is less than 1"),
        ] {
            let err = load(content).unwrap_err();
            assert!(err.contains(want), "{err}");
            assert!(err.contains("agents.json"), "{err}");
        }
    }

    #[test]
    fn null_values_read_as_absent() {
        let c = load(r#"{"kit":null,"setup":null,"flow":null,"profiles":{"x":{"argv":["a"],"agent":null}}}"#).unwrap();
        assert_eq!((c.flow, c.kit.as_str(), c.setup.len()), (Flow::default(), "", 0));
        assert_eq!(c.profiles["x"].agent, "");
    }

    #[test]
    fn no_profile_names_the_file_and_the_profiles() {
        let mut c = Config::default();
        assert_eq!(c.no_profile("x"), "agents: no profile \"x\"; profiles: builder, debugger, planner, reviewer");
        c.source = "/a.json".into();
        assert!(c.no_profile("x").starts_with("agents: no profile \"x\" in /a.json; profiles: "));
    }

    #[test]
    fn flow_lines_and_text_follow_the_rules() {
        let f = Flow::default();
        let lines = f.lines();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("Plan first: when needed."));
        assert!(lines[1].starts_with("Review: code PRs."));
        assert_eq!(lines[2], "Debugger: spawn debugger after 2 failed fix rounds.");
        assert_eq!(lines[3], "Workers at once: at most 4, reviewers included.");
        assert_eq!(lines[4], "Merge: ask the owner each time.");
        assert_eq!(f.text(), format!("The flow:\n- {}\n", lines.join("\n- ")));
        let g = Flow { plan: "never".into(), review: "every".into(), merge: "auto".into(), ..Flow::default() };
        let l = g.lines();
        assert_eq!(
            (l[0].as_str(), l[1].as_str(), l[4].as_str()),
            (
                "Plan first: never. Spawn builder directly; the builder plans in its PR body.",
                "Review: every PR gets a reviewer.",
                "Merge: merge yourself when CI and review pass."
            )
        );
    }

    #[test]
    fn save_keeps_unknown_keys_and_does_not_freeze_the_defaults() {
        let s = Scratch::new(Some(FULL));
        let mut c = Config::load(&s.path()).unwrap();
        c.flow.workers = 9;
        c.save(&s.path()).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(s.path()).unwrap()).unwrap();
        assert_eq!(saved["theme"]["accent"], "blue");
        assert_eq!(saved["flow"]["workers"], 9);
        // The file had `builder` and `planner-high`; the untouched defaults stay out.
        let names: Vec<_> = saved["profiles"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(names, ["builder", "planner-high"]);
        assert!(!s.0.join("agents.json.tmp").exists());
        assert_eq!(Config::load(&s.path()).unwrap().flow.workers, 9);
    }

    #[test]
    fn save_writes_a_changed_default_and_creates_the_file() {
        let s = Scratch::new(None);
        let path = s.0.join("sub/agents.json");
        let mut c = Config::default();
        c.profiles.get_mut("reviewer").unwrap().argv = vec!["claude".into(), "--model".into(), "haiku".into()];
        c.save(&path).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let names: Vec<_> = saved["profiles"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(names, ["reviewer"]);
        assert!(saved.get("kit").is_none() && saved.get("setup").is_none());
        assert_eq!(saved["flow"]["plan"], "when-needed");
        // An untouched default config writes no profiles key at all.
        let plain = s.0.join("plain.json");
        Config::default().save(&plain).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&plain).unwrap()).unwrap();
        assert!(saved.get("profiles").is_none());
    }

    #[test]
    fn save_refuses_a_bad_flow_and_a_file_that_is_not_json() {
        let s = Scratch::new(Some("not json"));
        assert!(Config::default().save(&s.path()).is_err());
        let s = Scratch::new(None);
        let mut c = Config::default();
        c.flow.merge = "maybe".into();
        assert!(c.save(&s.path()).unwrap_err().contains("flow merge"));
        assert!(!s.path().exists());
    }
}
