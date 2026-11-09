//! Starting and listing worker agents in hc terminals, Go-compatible (PARITY-PLAN K3,
//! go:internal/agents). A worker is an hc terminal with the label `contr=agent`. The CLI
//! (`contr-rune agents`) and the app's "new orchestrator" (A3) share `spawn`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::agents_config::{Config, flag_value};
use crate::hc::conn::Conn;
use crate::hc::wire::Terminal;
use crate::{github, prompt};

/// How long a call to the core may take. `terminal.run` does not wait for the program.
const CALL: Duration = Duration::from_secs(30);

/// `$0` of the setup wrapper; `command` finds the agent after it.
const SETUP_NAME: &str = "contr-setup";

/// Runs the setup command (`$1`) in the worktree, then starts the agent (the words after it). A
/// failed setup ends the terminal with the setup's exit code, so the parent gets a note and the
/// output stays readable.
const SETUP_WRAPPER: &str = r#"/bin/sh -c "$1" || { s=$?; echo "contr: worktree setup failed (exit $s)" >&2; exit $s; }; shift; exec "$@""#;

/// A worker to start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spec {
    /// Unique among live terminals.
    pub name: String,
    pub profile: String,
    /// Defaults to the profile name.
    pub role: String,
    pub issue: String,
    /// The working folder; absolute by the time `spawn_params` sees it.
    pub worktree: String,
    /// The first message, if any: the last argument of the agent.
    pub task: String,
    /// The hc terminal that gets notes about the worker; empty for none.
    pub parent: String,
    /// The project name; no control characters.
    pub project: String,
}

/// A setup command and the main checkout it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub command: String,
    pub main: String,
}

/// The `terminal.run` params for `spec` (K3). `kit` is the plugin folder; `setup` is what the
/// worktree's main checkout asks to run before the agent.
pub fn spawn_params(spec: &Spec, cfg: &Config, kit: Option<&Path>, setup: Option<&Setup>) -> Result<Value, String> {
    let profile = cfg.profiles.get(&spec.profile).ok_or_else(|| cfg.no_profile(&spec.profile))?;
    if spec.name.is_empty() {
        return Err("agents: a worker needs a name".into());
    }
    if spec.project.chars().any(char::is_control) {
        return Err("agents: the project name has a control character".into());
    }
    let role = if spec.role.is_empty() { &spec.profile } else { &spec.role };
    let mut labels = BTreeMap::from([("contr", "agent"), ("role", role), ("profile", &spec.profile), ("worktree", &spec.worktree)]);
    if !spec.project.is_empty() {
        labels.insert("project", &spec.project);
    }
    if !spec.issue.is_empty() {
        labels.insert("issue", &spec.issue);
    }
    let agent = if profile.agent.is_empty() { &spec.profile } else { &profile.agent };
    let mut argv = agent_argv(kit, agent, &profile.argv);
    if !spec.task.is_empty() {
        argv.push(spec.task.clone());
    }
    let mut env = BTreeMap::new();
    if let Some(s) = setup {
        argv = [["/bin/sh", "-c", SETUP_WRAPPER, SETUP_NAME, &s.command].map(String::from).to_vec(), argv].concat();
        env.insert("CONTR_MAIN", s.main.as_str());
    }
    if !spec.parent.is_empty() {
        env.insert("CONTR_COORDINATOR", &spec.parent);
    }
    let mut params = json!({"argv": argv, "name": spec.name, "labels": labels, "wait": false, "outlive_parent": true});
    if !spec.worktree.is_empty() {
        params["cwd"] = json!(spec.worktree);
    }
    if !env.is_empty() {
        params["env"] = json!(env);
    }
    if spec.parent.is_empty() {
        params["no_parent"] = json!(true);
    } else {
        params["parent"] = json!(spec.parent);
    }
    Ok(params)
}

/// The profile's command. A `claude` command whose agent has a definition in the kit gets
/// `--plugin-dir <kit> --agent contr:<agent>` after the program name.
fn agent_argv(kit: Option<&Path>, agent: &str, argv: &[String]) -> Vec<String> {
    let claude = argv.first().is_some_and(|a| Path::new(a).file_name().is_some_and(|n| n == "claude"));
    match kit {
        Some(kit) if claude && kit.join("agents").join(format!("{agent}.md")).exists() => {
            let mut out = vec![argv[0].clone(), "--plugin-dir".into(), kit.display().to_string(), "--agent".into(), format!("contr:{agent}")];
            out.extend_from_slice(&argv[1..]);
            out
        }
        _ => argv.to_vec(),
    }
}

/// The kit folder: `CONTR_KIT`, else the config's `kit`, else the one bundled in the app
/// (`Contents/Resources/kit` next to `Contents/MacOS/<exe>`). A relative path is made absolute,
/// because the agent runs in its worktree.
pub fn kit_dir(env_kit: Option<&OsString>, cfg: &Config, exe: Option<&Path>) -> Option<PathBuf> {
    let chosen = env_kit.filter(|k| !k.is_empty()).map(PathBuf::from).or_else(|| (!cfg.kit.is_empty()).then(|| PathBuf::from(&cfg.kit)));
    let bundled = || exe?.parent()?.parent().map(|c| c.join("Resources/kit")).filter(|k| k.is_dir());
    chosen.map(|k| std::path::absolute(&k).unwrap_or(k)).or_else(bundled)
}

/// `dir` as an absolute path to a folder: hc resolves a relative `cwd` from its own folder.
pub fn worktree_dir(dir: &str) -> Result<String, String> {
    let abs = std::path::absolute(dir).map_err(|e| format!("agents: worktree {dir}: {e}"))?;
    match std::fs::metadata(&abs) {
        Ok(m) if m.is_dir() => Ok(abs.display().to_string()),
        Ok(_) => Err(format!("agents: worktree {} is not a folder", abs.display())),
        Err(e) => Err(format!("agents: worktree {}: {e}", abs.display())),
    }
}

/// The main checkout for a git common dir: its parent when the folder is named `.git`.
fn main_checkout(common_dir: &str) -> Option<PathBuf> {
    let common = Path::new(common_dir);
    (common.file_name()? == ".git").then(|| common.parent().map(Path::to_path_buf))?
}

/// `~/x` with `home` for the `~`.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.canonicalize().unwrap_or_else(|_| a.to_path_buf()) == b.canonicalize().unwrap_or_else(|_| b.to_path_buf())
}

/// The setup command of the project that `dir` is a worktree of, and its main checkout. The main
/// checkout itself gets none (the owner prepares it), nor does a folder outside git.
fn setup_for(cfg: &Config, dir: &str, home: Option<&Path>) -> Option<Setup> {
    if cfg.setup.is_empty() || dir.is_empty() {
        return None;
    }
    let out = Command::new("git").args(["-C", dir, "rev-parse", "--path-format=absolute", "--git-common-dir"]).output().ok().filter(|o| o.status.success())?;
    let main = main_checkout(String::from_utf8_lossy(&out.stdout).trim())?;
    if same_path(&main, Path::new(dir)) {
        return None;
    }
    let (_, command) = cfg.setup.iter().find(|(k, _)| same_path(&expand_home(k, home), &main))?;
    Some(Setup { command: command.clone(), main: main.display().to_string() })
}

/// Starts a worker on `conn` and returns it. `env_kit` is `CONTR_KIT`, `home` is `$HOME`.
pub fn spawn(conn: &Conn, cfg: &Config, mut spec: Spec, env_kit: Option<&OsString>, home: Option<&Path>) -> Result<Worker, String> {
    if !spec.worktree.is_empty() {
        spec.worktree = worktree_dir(&spec.worktree)?;
    }
    let kit = kit_dir(env_kit, cfg, std::env::current_exe().ok().as_deref());
    let setup = setup_for(cfg, &spec.worktree, home);
    let params = spawn_params(&spec, cfg, kit.as_deref(), setup.as_ref())?;
    let raw = conn.call("terminal.run", params, CALL).map_err(|e| e.to_string())?;
    Ok(Worker::from(&terminal_of(raw)?))
}

/// The workers (`contr=agent` terminals), sorted by name then address. With `all`, finished ones
/// too.
pub fn list(conn: &Conn, all: bool) -> Result<Vec<Worker>, String> {
    let raw = conn.call("terminal.list", json!({"all": all, "labels": {"contr": "agent"}}), CALL).map_err(|e| e.to_string())?;
    let mut workers = Vec::new();
    for t in raw["terminals"].as_array().into_iter().flatten() {
        workers.push(Worker::from(&terminal_of(t.clone())?));
    }
    workers.sort_by(|a, b| (&a.name, &a.address).cmp(&(&b.name, &b.address)));
    Ok(workers)
}

/// The params of a call that looks at, acts on or sends to a terminal. Inside hc they name this
/// terminal as the `caller`: when it is the worker's parent, hc counts the worker's news as seen
/// and types no note about it into the parent.
fn looking(name: &str, caller: &str) -> Value {
    let mut params = json!({"terminal": name});
    if !caller.is_empty() {
        params["caller"] = json!(caller);
    }
    params
}

fn call(conn: &Conn, method: &str, params: Value) -> Result<Value, String> {
    conn.call(method, params, CALL).map_err(|e| e.to_string())
}

/// Types `text` into a worker and presses Enter. Blank text is an error (hc would still submit
/// its sender tag, an empty message), and so is a send hc refuses.
pub fn send(conn: &Conn, caller: &str, name: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("agents: no text to send".into());
    }
    let mut params = looking(name, caller);
    params["text"] = json!(text);
    let res = call(conn, "terminal.send", params)?;
    if res["sent"] == json!(false) {
        let reason = res["reason"].as_str().filter(|r| !r.is_empty()).unwrap_or("no reason given");
        return Err(format!("agents: {name} did not take the text: {reason}"));
    }
    Ok(())
}

/// Presses a worker's stop key. The outcome is hc's: `stopped`, `exited`, `still_working` or
/// `unknown`.
pub fn interrupt(conn: &Conn, caller: &str, name: &str) -> Result<String, String> {
    let res = call(conn, "terminal.interrupt", looking(name, caller))?;
    Ok(res["outcome"].as_str().unwrap_or_default().to_owned())
}

/// One worker by name or address.
fn get(conn: &Conn, caller: &str, name: &str) -> Result<Worker, String> {
    Ok(Worker::from(&terminal_of(call(conn, "terminal.get", looking(name, caller))?)?))
}

/// Deletes a finished terminal from hc. hc refuses a live one.
pub fn remove(conn: &Conn, caller: &str, name: &str) -> Result<(), String> {
    call(conn, "terminal.rm", looking(name, caller)).map(drop)
}

/// Ends a worker's program, then removes its terminal. With `keep` the finished terminal stays
/// for reading. A terminal that already finished is not killed again.
pub fn stop(conn: &Conn, caller: &str, name: &str, keep: bool) -> Result<(), String> {
    if get(conn, caller, name)?.live {
        call(conn, "terminal.kill", looking(name, caller))?;
    }
    if keep { Ok(()) } else { remove(conn, caller, name) }
}

/// Removes the finished workers that `caller` started (those whose parent it is). Orchestrators,
/// plain terminals and other coordinators' workers stay; outside hc (no `caller`) nothing goes.
/// Returns the names removed, and the error that stopped the removal, if any.
pub fn remove_ended(conn: &Conn, caller: &str) -> (Vec<String>, Option<String>) {
    let mut removed = Vec::new();
    if caller.is_empty() {
        return (removed, None);
    }
    let workers = match list(conn, true) {
        Ok(w) => w,
        Err(e) => return (removed, Some(e)),
    };
    for w in workers.iter().filter(|w| !w.live && w.role != "orchestrator" && w.role != "terminal" && w.parent == caller) {
        if let Err(e) = remove(conn, caller, &w.address) {
            return (removed, Some(format!("remove {}: {e}", w.name)));
        }
        removed.push(w.name.clone());
    }
    (removed, None)
}

/// A worker's screen and history; `lines > 0` keeps only the last lines.
pub fn read(conn: &Conn, caller: &str, name: &str, lines: i64) -> Result<String, String> {
    let mut params = looking(name, caller);
    if lines > 0 {
        params["lines"] = json!(lines);
    }
    Ok(call(conn, "terminal.read", params)?["text"].as_str().unwrap_or_default().to_owned())
}

/// What `agents prompt` prints when the worker waits on no menu.
const NO_PROMPT: &str = "no prompt";

/// How many terminal lines `pending_prompt` reads to find the menu.
const PROMPT_LINES: i64 = 40;

/// The menu a blocked worker waits on: the detail, the question and the numbered choices; or
/// `no prompt` when it is not blocked or its screen has no menu we can read.
pub fn pending_prompt(conn: &Conn, caller: &str, name: &str) -> Result<String, String> {
    let w = get(conn, caller, name)?;
    if !w.live || w.status != "blocked" {
        return Ok(NO_PROMPT.into());
    }
    let screen = read(conn, caller, name, PROMPT_LINES)?;
    let Some(p) = prompt::parse(&w.command, &w.kind, &screen) else { return Ok(NO_PROMPT.into()) };
    let mut out: Vec<String> = p.detail;
    out.push(p.question);
    out.extend(p.choices.iter().enumerate().map(|(i, c)| format!("{}. {c}", i + 1)));
    Ok(out.join("\n").trim_end_matches('\n').to_owned())
}

/// Runs git in `dir`: its trimmed stdout, or `git <verb>: <stderr>`.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    }
    let msg = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(format!("git {}: {}", args[0], if msg.is_empty() { out.status.to_string() } else { msg }))
}

/// A detached worktree at the head of pull request `pr` of the repository at `repo` (any of its
/// checkouts): `<main checkout>-wt/review-<pr>`. An existing one moves to the new head.
pub fn review_worktree(repo: &str, pr: i64) -> Result<String, String> {
    if pr <= 0 {
        return Err("agents: a pull request number is positive".into());
    }
    let common = git(Path::new(repo), &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let main = main_checkout(&common).ok_or_else(|| format!("agents: {repo} has no main checkout (git folder {common})"))?;
    let sha = github::pr_head(&main, pr).map_err(|e| format!("agents: pull request {pr}: {e}"))?;
    if git(&main, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_err() {
        git(&main, &["fetch", "--quiet", "origin", &format!("refs/pull/{pr}/head")])?;
    }
    let dir = PathBuf::from(format!("{}-wt/review-{pr}", main.display()));
    if dir.exists() {
        git(&dir, &["checkout", "--quiet", "--detach", &sha])?;
    } else {
        git(&main, &["worktree", "add", "--quiet", "--detach", &dir.display().to_string(), &sha])?;
    }
    Ok(dir.display().to_string())
}

/// A terminal record from the core. A `null` field (a run without `cwd`, say) counts as absent.
fn terminal_of(mut raw: Value) -> Result<Terminal, String> {
    if let Value::Object(fields) = &mut raw {
        fields.retain(|_, v| !v.is_null());
    }
    serde_json::from_value(raw).map_err(|e| format!("agents: bad terminal: {e}"))
}

/// One worker as `agents ls --json` and `agents spawn --json` print it (Go's field names).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Worker {
    pub name: String,
    pub address: String,
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub issue: String,
    pub profile: String,
    pub worktree: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub project: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// The program that runs now: hc's foreground name, else the base name of the spawn command.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// False once the program ended.
    pub live: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// `permission`, `question` or `auth`, when blocked.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub msg: String,
    pub turn: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub orchestrating: bool,
}

impl From<&Terminal> for Worker {
    fn from(t: &Terminal) -> Worker {
        let label = |k: &str| t.labels.get(k).cloned().unwrap_or_default();
        let or = |a: String, b: String| if a.is_empty() { b } else { a };
        let status = t.status.as_ref();
        let in_front = t.foreground.as_ref().is_some_and(|f| {
            let base = Path::new(&f.name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            base == "claude" || is_version(&base)
        });
        // The owner ran Claude Code in a plain terminal: while it runs, it is a Claude session.
        let role = if label("role") == "terminal" && in_front { "orchestrator".to_owned() } else { label("role") };
        Worker {
            name: t.name.clone(),
            address: t.address.clone(),
            role,
            issue: label("issue"),
            profile: label("profile"),
            worktree: label("worktree"),
            project: label("project"),
            parent: t.parent.clone().unwrap_or_default(),
            command: command(t),
            live: t.state != "finished",
            status: status.map(|s| s.state.clone()).unwrap_or_default(),
            kind: status.and_then(|s| s.kind.clone()).unwrap_or_default(),
            msg: status.map(|s| s.msg.clone()).unwrap_or_default(),
            turn: t.turn,
            title: t.title.clone(),
            exit_code: t.exit_code,
            model: or(flag_value(&t.argv, "--model"), label("contr.model")),
            effort: or(flag_value(&t.argv, "--effort"), label("contr.effort")),
            orchestrating: !label("contr.orchestrating").is_empty(),
        }
    }
}

/// Claude Code names its foreground process by version, such as `2.1.30`.
fn is_version(name: &str) -> bool {
    let mut parts = name.splitn(3, '.');
    let digits = |p: Option<&str>| p.is_some_and(|p| p.starts_with(|c: char| c.is_ascii_digit()));
    digits(parts.next()) && parts.next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) && digits(parts.next())
}

/// The foreground program's name, else the base of `argv[0]` (of the agent's after a setup
/// wrapper).
fn command(t: &Terminal) -> String {
    if let Some(f) = t.foreground.as_ref().filter(|f| !f.name.is_empty()) {
        return f.name.clone();
    }
    let argv = if t.argv.len() > 5 && t.argv[3] == SETUP_NAME { &t.argv[5..] } else { &t.argv[..] };
    argv.first().and_then(|a| Path::new(a).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The state column: how it ended, why it is blocked, or the program's status.
pub fn state_text(w: &Worker) -> String {
    match (w.live, w.exit_code) {
        (false, Some(code)) => format!("ended ({code})"),
        (false, None) => "ended".into(),
        _ if w.status == "blocked" && !w.kind.is_empty() => format!("blocked: {}", w.kind),
        _ if !w.status.is_empty() => w.status.clone(),
        _ => "-".into(),
    }
}

/// The `agents ls` table, aligned as Go's tabwriter does: two spaces after the longest cell of a
/// column, no padding after the last column.
pub fn table(workers: &[Worker]) -> String {
    let mut rows = vec![["NAME", "ADDRESS", "ROLE", "ISSUE", "STATE", "WORKTREE"].map(String::from)];
    for w in workers {
        let issue = if w.issue.is_empty() { "-".to_owned() } else { w.issue.clone() };
        rows.push([w.name.clone(), w.address.clone(), w.role.clone(), issue, state_text(w), w.worktree.clone()]);
    }
    let widths: Vec<usize> = (0..5).map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0) + 2).collect();
    let mut out = String::new();
    for r in &rows {
        for (c, cell) in r.iter().enumerate() {
            out.push_str(cell);
            if c < 5 {
                out.extend(std::iter::repeat_n(' ', widths[c] - cell.chars().count()));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents_config::Profile;

    fn spec() -> Spec {
        Spec { name: "b-1".into(), profile: "builder".into(), worktree: "/w".into(), ..Spec::default() }
    }

    fn scratch_kit(agents: &[&str]) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let kit = std::env::temp_dir().join(format!("contr-rune-kit-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&kit);
        std::fs::create_dir_all(kit.join("agents")).unwrap();
        for a in agents {
            std::fs::write(kit.join("agents").join(format!("{a}.md")), "").unwrap();
        }
        kit
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_plain_spawn_has_the_labels_and_no_parent() {
        let p = spawn_params(&spec(), &Config::default(), None, None).unwrap();
        assert_eq!(
            p,
            json!({
                "argv": ["claude", "--model", "sonnet", "--effort", "medium", "--permission-mode", "auto"],
                "name": "b-1",
                "labels": {"contr": "agent", "role": "builder", "profile": "builder", "worktree": "/w"},
                "wait": false, "outlive_parent": true, "cwd": "/w", "no_parent": true
            })
        );
    }

    #[test]
    fn parent_role_issue_project_and_task_are_passed_on() {
        let s = Spec { parent: "t3".into(), role: "fixer".into(), issue: "241".into(), project: "contr".into(), task: "do it".into(), ..spec() };
        let p = spawn_params(&s, &Config::default(), None, None).unwrap();
        assert_eq!(p["labels"], json!({"contr": "agent", "role": "fixer", "profile": "builder", "worktree": "/w", "issue": "241", "project": "contr"}));
        assert_eq!((p["parent"].as_str(), p["env"].clone(), p.get("no_parent")), (Some("t3"), json!({"CONTR_COORDINATOR": "t3"}), None));
        assert_eq!(p["argv"].as_array().unwrap().last().unwrap(), "do it");
        // No worktree: no `cwd`, and the label stays empty as in Go.
        let p = spawn_params(&Spec { worktree: String::new(), ..spec() }, &Config::default(), None, None).unwrap();
        assert_eq!((p.get("cwd"), p["labels"]["worktree"].as_str()), (None, Some("")));
    }

    #[test]
    fn a_claude_profile_with_a_kit_agent_starts_as_that_agent() {
        let kit = scratch_kit(&["builder", "planner"]);
        let k = kit.display().to_string();
        let argv = |name: &str, cfg: &Config, kit: Option<&Path>| {
            let p = spawn_params(&Spec { profile: name.into(), ..spec() }, cfg, kit, None).unwrap();
            serde_json::from_value::<Vec<String>>(p["argv"].clone()).unwrap()
        };
        let cfg = Config::default();
        let got = argv("builder", &cfg, Some(&kit));
        assert_eq!(got[..5], strings(&["claude", "--plugin-dir", &k, "--agent", "contr:builder"])[..]);
        assert_eq!(got[5..7], strings(&["--model", "sonnet"])[..]);
        // No definition for the reviewer, and no kit at all: the argv stays as the profile has it.
        assert_eq!(argv("reviewer", &cfg, Some(&kit))[1], "--model");
        assert_eq!(argv("builder", &cfg, None)[1], "--model");
        // A variant names its agent; a program other than claude is left alone.
        let mut cfg = Config::default();
        cfg.profiles.insert("planner-high".into(), Profile { argv: strings(&["/usr/bin/claude", "--effort", "high"]), agent: "planner".into() });
        cfg.profiles.insert("other".into(), Profile { argv: strings(&["codex"]), agent: "builder".into() });
        assert_eq!(argv("planner-high", &cfg, Some(&kit))[..5], strings(&["/usr/bin/claude", "--plugin-dir", &k, "--agent", "contr:planner"])[..]);
        assert_eq!(argv("other", &cfg, Some(&kit)), ["codex"]);
        std::fs::remove_dir_all(kit).unwrap();
    }

    #[test]
    fn a_setup_wraps_the_argv_and_sets_the_main_checkout() {
        let setup = Setup { command: "npm ci".into(), main: "/m".into() };
        let s = Spec { parent: "t3".into(), task: "go".into(), ..spec() };
        let p = spawn_params(&s, &Config::default(), None, Some(&setup)).unwrap();
        let argv: Vec<String> = serde_json::from_value(p["argv"].clone()).unwrap();
        assert_eq!(argv[..5], ["/bin/sh", "-c", SETUP_WRAPPER, "contr-setup", "npm ci"]);
        assert_eq!((argv[5].as_str(), argv.last().unwrap().as_str()), ("claude", "go"));
        assert_eq!(p["env"], json!({"CONTR_MAIN": "/m", "CONTR_COORDINATOR": "t3"}));
    }

    #[test]
    fn bad_specs_are_refused() {
        let cfg = Config::default();
        let err = |s: Spec| spawn_params(&s, &cfg, None, None).unwrap_err();
        assert!(err(Spec { profile: "nobody".into(), ..spec() }).starts_with("agents: no profile \"nobody\""));
        assert_eq!(err(Spec { name: String::new(), ..spec() }), "agents: a worker needs a name");
        assert_eq!(err(Spec { project: "a\nb".into(), ..spec() }), "agents: the project name has a control character");
    }

    #[test]
    fn the_kit_comes_from_the_variable_then_the_config_then_the_app_bundle() {
        let bundle = std::env::temp_dir().join(format!("contr-rune-bundle-{}/Contents", std::process::id()));
        std::fs::create_dir_all(bundle.join("Resources/kit")).unwrap();
        let exe = bundle.join("MacOS/contr-rune");
        let mut cfg = Config::default();
        assert_eq!(kit_dir(None, &cfg, Some(&exe)), Some(bundle.join("Resources/kit")));
        assert_eq!(kit_dir(None, &cfg, Some(Path::new("/nowhere/contr-rune"))), None);
        cfg.kit = "/from/config".into();
        assert_eq!(kit_dir(None, &cfg, Some(&exe)), Some("/from/config".into()));
        assert_eq!(kit_dir(Some(&"/from/env".into()), &cfg, Some(&exe)), Some("/from/env".into()));
        assert_eq!(kit_dir(Some(&"".into()), &cfg, None), Some("/from/config".into()));
        assert!(kit_dir(Some(&"rel/kit".into()), &cfg, None).unwrap().is_absolute());
        std::fs::remove_dir_all(bundle.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_main_checkout_and_the_setup_key_are_found_by_path() {
        assert_eq!(main_checkout("/r/app/.git"), Some("/r/app".into()));
        assert_eq!(main_checkout("/r/app.git"), None);
        assert_eq!(expand_home("~/code/app", Some(Path::new("/h"))), PathBuf::from("/h/code/app"));
        assert_eq!(expand_home("~/code/app", None), PathBuf::from("~/code/app"));
        assert_eq!(expand_home("/abs", Some(Path::new("/h"))), PathBuf::from("/abs"));
    }

    #[test]
    fn a_worktree_must_be_an_existing_folder() {
        let here = std::env::current_dir().unwrap().display().to_string();
        assert_eq!(worktree_dir(".").unwrap().trim_end_matches("/."), here);
        assert!(worktree_dir("/nonexistent-contr-dir").unwrap_err().starts_with("agents: worktree /nonexistent-contr-dir: "));
        assert!(worktree_dir("Cargo.toml").unwrap_err().ends_with("Cargo.toml is not a folder"));
    }

    fn terminal(v: Value) -> Terminal {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_worker_reads_labels_status_and_the_command_from_the_terminal() {
        let t = terminal(json!({
            "address": "t17", "name": "b-241", "parent": "t3", "state": "live", "turn": 4, "title": "Build",
            "argv": ["/bin/sh", "-c", "w", "contr-setup", "npm ci", "claude", "--model", "sonnet"],
            "labels": {"contr": "agent", "role": "builder", "issue": "241", "contr.effort": "high"},
            "status": {"state": "blocked", "kind": "permission", "msg": "Run?"}
        }));
        let w = Worker::from(&t);
        assert_eq!((w.command.as_str(), w.model.as_str(), w.effort.as_str(), w.parent.as_str()), ("claude", "sonnet", "high", "t3"));
        assert_eq!(state_text(&w), "blocked: permission");
        let j = serde_json::to_value(&w).unwrap();
        assert_eq!(j["issue"], "241");
        assert!(j.get("project").is_none() && j.get("exit_code").is_none() && j.get("orchestrating").is_none());
    }

    #[test]
    fn a_null_field_in_a_terminal_counts_as_absent() {
        let t = terminal_of(json!({"address": "t1", "cwd": null, "parent": null, "labels": {"role": "builder"}})).unwrap();
        assert_eq!((t.cwd.as_str(), t.parent), ("", None));
        assert!(terminal_of(json!({"cwd": "/w"})).unwrap_err().starts_with("agents: bad terminal: "));
    }

    #[test]
    fn the_state_text_follows_live_exit_code_and_status() {
        let w = |live, exit_code, status: &str, kind: &str| Worker { live, exit_code, status: status.into(), kind: kind.into(), ..Worker::from(&terminal(json!({"address": "t1"}))) };
        assert_eq!(state_text(&w(false, Some(2), "", "")), "ended (2)");
        assert_eq!(state_text(&w(false, None, "idle", "")), "ended");
        assert_eq!(state_text(&w(true, None, "blocked", "")), "blocked");
        assert_eq!(state_text(&w(true, None, "working", "")), "working");
        assert_eq!(state_text(&w(true, None, "", "")), "-");
    }

    #[test]
    fn claude_in_front_of_a_plain_terminal_makes_it_an_orchestrator() {
        let t = |fg: &str| terminal(json!({"address": "t1", "state": "live", "labels": {"contr": "agent", "role": "terminal"}, "foreground": {"name": fg}}));
        assert_eq!(Worker::from(&t("claude")).role, "orchestrator");
        assert_eq!(Worker::from(&t("2.1.30")).role, "orchestrator");
        assert_eq!(Worker::from(&t("zsh")).role, "terminal");
        assert_eq!(Worker::from(&t("zsh")).command, "zsh");
    }

    #[test]
    fn the_table_aligns_columns_with_two_spaces_and_leaves_the_last_free() {
        let t = |name: &str, state: &str, issue: &str| {
            Worker::from(&terminal(json!({
                "address": "t1", "name": name, "state": state, "exit_code": if state == "finished" { json!(0) } else { Value::Null },
                "labels": {"role": "builder", "issue": issue, "worktree": "/w"}
            })))
        };
        let out = table(&[t("b-1", "live", ""), t("builder-long", "finished", "7")]);
        assert_eq!(
            out,
            "NAME          ADDRESS  ROLE     ISSUE  STATE      WORKTREE\n\
             b-1           t1       builder  -      -          /w\n\
             builder-long  t1       builder  7      ended (0)  /w\n"
        );
    }
}
