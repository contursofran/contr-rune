//! `contr-rune agents ls | spawn | flow` (PARITY-PLAN K4; Go: cmd/contr/agents.go). Exit codes:
//! 0 ok, 1 error, 2 usage. The other subcommands arrive with D4b.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use crate::agents::{self, Spec};
use crate::agents_config::{Config, Flow};
use crate::hc::conn::Conn;
use crate::hc::dial::{Dialer, Keep};
use crate::hc::watch::{HcStatus, handshake};
use crate::paths;

const USAGE: &str = "\
Usage:
  contr-rune agents ls [--all] [--json]            list the workers
  contr-rune agents spawn --name N --profile P [--issue I] [--worktree DIR] [--project P]
                     [--role R] [--no-parent] [--json]
                     [--task-file F | [--] task…]
                                                   start a worker; --task-file reads the
                                                   task from a file
  contr-rune agents flow                           print the flow: how the orchestrator takes an issue to a merge

Flags can come before or after the other arguments. After --, every word is
part of the task.

Environment:
  CONTR_AGENTS_CONFIG   profiles file (default: ~/Library/Application Support/contr/agents.json):
                        {\"kit\": DIR, \"profiles\": {NAME: {\"argv\": [...], \"agent\": KIT_AGENT}},
                         \"setup\": {MAIN_CHECKOUT: SHELL_COMMAND},
                         \"flow\": {\"plan\": \"never|when-needed|always\", \"review\": \"none|code|every\",
                                  \"debugger_after\": N, \"workers\": N, \"merge\": \"ask|auto\"}}
                        setup runs in a new worktree of that main checkout before the
                        agent starts, with CONTR_MAIN set to the main checkout
  CONTR_HC              the hc program (default: hc from PATH, then ~/.local/bin, /opt/homebrew/bin, /usr/local/bin)
  CONTR_KIT             contr's Claude Code plugin folder (the repository's kit);
                        claude profiles with an agent there start as that agent
  HC_TERMINAL_ID        set inside hc: spawn makes that terminal the parent
";

/// The environment a call reads, so tests need not touch the process environment.
pub struct Env {
    pub config_path: Option<PathBuf>,
    pub kit: Option<OsString>,
    pub home: Option<PathBuf>,
    pub terminal_id: String,
}

impl Env {
    pub fn real() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Env {
            config_path: paths::agents_config(),
            kit: var("CONTR_KIT"),
            home: var("HOME").map(PathBuf::from),
            terminal_id: var("HC_TERMINAL_ID").map(|v| v.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
}

/// `(name, takes a value, subcommands it applies to)`.
const FLAGS: [(&str, bool, &[&str]); 10] = [
    ("all", false, &["ls"]),
    ("json", false, &["ls", "spawn"]),
    ("name", true, &["spawn"]),
    ("profile", true, &["spawn"]),
    ("issue", true, &["spawn"]),
    ("worktree", true, &["spawn"]),
    ("project", true, &["spawn"]),
    ("role", true, &["spawn"]),
    ("no-parent", false, &["spawn"]),
    ("task-file", true, &["spawn"]),
];

/// The flags and the other arguments of one subcommand call.
#[derive(Debug, Default, PartialEq)]
struct Parsed {
    flags: BTreeMap<&'static str, String>,
    pos: Vec<String>,
}

impl Parsed {
    fn flag(&self, name: &str) -> bool {
        self.flags.get(name).is_some_and(|v| v == "true")
    }

    fn text(&self, name: &str) -> String {
        self.flags.get(name).cloned().unwrap_or_default()
    }
}

/// Parses flags before and after the other arguments, as Go's `flag` package does (`-x` or `--x`,
/// `--x=v` or `--x v`). After `--` every word is an argument.
fn parse(sub: &str, args: &[String]) -> Result<Parsed, String> {
    let (args, tail) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[][..]),
    };
    let mut parsed = Parsed::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(body) = arg.strip_prefix('-').filter(|b| !b.is_empty()) else {
            parsed.pos.push(arg.clone());
            continue;
        };
        let body = body.strip_prefix('-').unwrap_or(body);
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (body, None),
        };
        let &(flag, takes, applies) = FLAGS.iter().find(|(n, ..)| *n == name).ok_or_else(|| format!("flag provided but not defined: -{name}"))?;
        if !applies.contains(&sub) {
            return Err(format!("flag --{flag} does not apply"));
        }
        let value = match (takes, inline) {
            (true, Some(v)) => v,
            (true, None) => it.next().cloned().ok_or_else(|| format!("flag needs an argument: -{flag}"))?,
            (false, None) => "true".to_owned(),
            (false, Some(v)) => match v.as_str() {
                "true" | "1" | "t" | "T" | "TRUE" | "True" => "true".to_owned(),
                "false" | "0" | "f" | "F" | "FALSE" | "False" => "false".to_owned(),
                _ => return Err(format!("invalid boolean value {v:?} for -{flag}")),
            },
        };
        parsed.flags.insert(flag, value);
    }
    parsed.pos.extend(tail.iter().cloned());
    Ok(parsed)
}

/// The task of a spawn: the text, or the content of `file`. Both at once, a missing file and a
/// file of blanks are errors, so a quoting slip never starts a worker with a wrong brief.
fn spawn_task(text: &str, file: &str) -> Result<String, String> {
    if file.is_empty() {
        return Ok(text.to_owned());
    }
    if !text.trim().is_empty() {
        return Err("give the task as text or with a task file, not both".into());
    }
    let body = std::fs::read_to_string(file).map_err(|e| format!("task file: {e}"))?;
    if body.trim().is_empty() {
        return Err(format!("task file {file} is empty"));
    }
    Ok(body)
}

fn spec_of(p: &Parsed, terminal_id: &str) -> Result<Spec, String> {
    Ok(Spec {
        name: p.text("name"),
        profile: p.text("profile"),
        role: p.text("role"),
        issue: p.text("issue"),
        worktree: p.text("worktree"),
        project: p.text("project"),
        task: spawn_task(&p.pos.join(" "), &p.text("task-file"))?,
        parent: if p.flag("no-parent") { String::new() } else { terminal_id.to_owned() },
    })
}

/// A line to the core. The child process of `hc connect` lives as long as the `Keep`.
fn connect() -> Result<(Conn, Keep), String> {
    let dialer = Dialer::from_env(false).map_err(|s| match s {
        HcStatus::Off => "hc is off (CONTR_HC=off)".to_owned(),
        HcStatus::NotInstalled => "hc is not installed (set CONTR_HC to its path)".to_owned(),
        HcStatus::Failed(e) => e,
        other => other.label(),
    })?;
    let link = dialer.dial().map_err(|e| e.to_string())?;
    let conn = Conn::new(link.reader, link.writer, None);
    handshake(&conn, false).map_err(|e| link.keep.stderr_tail().map_or(e.clone(), |t| format!("{e}: {t}")))?;
    Ok((conn, link.keep))
}

/// The flow block of the profiles file, or the default flow with the error when the file cannot
/// be read.
fn flow_text(env: &Env) -> String {
    match load(env) {
        Ok(cfg) => cfg.flow.text(),
        Err(e) => format!("The flow could not be read ({e}); the default flow follows.\n{}", Flow::default().text()),
    }
}

fn load(env: &Env) -> Result<Config, String> {
    match &env.config_path {
        Some(path) => Config::load(path),
        None => Err("agents: no home folder to find agents.json in".into()),
    }
}

/// Runs `contr-rune agents <args>` and returns the exit code.
pub fn run(args: &[String], env: &Env, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let Some(sub) = args.first().map(String::as_str) else {
        let _ = err.write_all(USAGE.as_bytes());
        return 2;
    };
    let rest = &args[1..];
    match sub {
        "help" | "-h" | "--help" => {
            let _ = write!(out, "{USAGE}\n{}", flow_text(env));
            0
        }
        "flow" if !rest.is_empty() => {
            let _ = writeln!(err, "contr-rune agents flow: takes no arguments");
            2
        }
        "flow" => match load(env) {
            Ok(cfg) => {
                let _ = out.write_all(cfg.flow.text().as_bytes());
                0
            }
            Err(e) => fail(err, &format!("contr-rune agents: {e}")),
        },
        "ls" | "spawn" => {
            let parsed = match parse(sub, rest) {
                Ok(p) => p,
                Err(e) => {
                    let _ = write!(err, "contr-rune agents {sub}: {e}\n{USAGE}");
                    return 2;
                }
            };
            if sub == "ls" && !parsed.pos.is_empty() {
                let _ = write!(err, "contr-rune agents ls: wrong number of arguments\n{USAGE}");
                return 2;
            }
            let result = if sub == "ls" { ls(&parsed, out) } else { spawn(&parsed, env, out) };
            match result {
                Ok(()) => 0,
                Err(Failure::Setup(e)) => fail(err, &format!("contr-rune agents: {e}")),
                Err(Failure::Run(e)) => fail(err, &format!("contr-rune agents {sub}: {e}")),
            }
        }
        "send" | "interrupt" | "stop" | "rm" | "read" | "prompt" => {
            let _ = writeln!(err, "contr-rune agents {sub}: not built yet");
            2
        }
        other => {
            let _ = write!(err, "contr-rune agents: unknown command {other}\n{USAGE}");
            2
        }
    }
}

fn fail(err: &mut impl Write, msg: &str) -> i32 {
    let _ = writeln!(err, "{msg}");
    1
}

/// Why a subcommand failed: before it ran (config, hc), or while it ran.
enum Failure {
    Setup(String),
    Run(String),
}

fn ls(p: &Parsed, out: &mut impl Write) -> Result<(), Failure> {
    let (conn, _keep) = connect().map_err(Failure::Setup)?;
    let workers = agents::list(&conn, p.flag("all")).map_err(Failure::Run)?;
    let text = if p.flag("json") { json_text(&workers)? } else { agents::table(&workers) };
    out.write_all(text.as_bytes()).map_err(|e| Failure::Run(e.to_string()))
}

fn spawn(p: &Parsed, env: &Env, out: &mut impl Write) -> Result<(), Failure> {
    let cfg = load(env).map_err(Failure::Setup)?;
    let spec = spec_of(p, &env.terminal_id).map_err(Failure::Run)?;
    let (conn, _keep) = connect().map_err(Failure::Setup)?;
    let worker = agents::spawn(&conn, &cfg, spec, env.kit.as_ref(), env.home.as_deref()).map_err(Failure::Run)?;
    let text = if p.flag("json") { json_text(&worker)? } else { format!("{} {}\n", worker.name, worker.address) };
    out.write_all(text.as_bytes()).map_err(|e| Failure::Run(e.to_string()))
}

/// Indented JSON and a newline, as Go's encoder writes it.
fn json_text(v: &impl serde::Serialize) -> Result<String, Failure> {
    serde_json::to_string_pretty(v).map(|s| s + "\n").map_err(|e| Failure::Run(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn flags_come_before_or_after_the_task_words() {
        let p = parse("spawn", &words("--name b-1 fix the bug --profile=builder -no-parent --json")).unwrap();
        assert_eq!(p.text("name"), "b-1");
        assert_eq!(p.text("profile"), "builder");
        assert!(p.flag("no-parent") && p.flag("json"));
        assert_eq!(p.pos, ["fix", "the", "bug"]);
    }

    #[test]
    fn after_a_double_dash_every_word_is_the_task() {
        let p = parse("spawn", &words("--name b -- --json is a word")).unwrap();
        assert!(!p.flag("json"));
        assert_eq!(p.pos, ["--json", "is", "a", "word"]);
        // A lone dash and a value that looks like a flag.
        let p = parse("spawn", &words("- --issue -5")).unwrap();
        assert_eq!((p.pos.as_slice(), p.text("issue")), (&["-".to_owned()][..], "-5".to_owned()));
    }

    #[test]
    fn a_bool_flag_takes_an_inline_value() {
        assert!(!parse("ls", &words("--all=false")).unwrap().flag("all"));
        assert!(parse("ls", &words("--all=true")).unwrap().flag("all"));
        assert!(parse("ls", &words("--all=maybe")).unwrap_err().starts_with("invalid boolean value"));
    }

    #[test]
    fn bad_flags_are_usage_errors() {
        for (sub, args, want) in [
            ("ls", "--name x", "flag --name does not apply"),
            ("spawn", "--all", "flag --all does not apply"),
            ("spawn", "--bogus", "flag provided but not defined: -bogus"),
            ("spawn", "--name", "flag needs an argument: -name"),
        ] {
            assert_eq!(parse(sub, &words(args)).unwrap_err(), want);
        }
    }

    fn scratch_file(content: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("contr-rune-task-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn the_task_is_the_words_or_a_file_never_both_and_never_blank() {
        assert_eq!(spawn_task("a b", "").unwrap(), "a b");
        let f = scratch_file("from file\n");
        let path = f.display().to_string();
        assert_eq!(spawn_task("", &path).unwrap(), "from file\n");
        assert_eq!(spawn_task("  ", &path).unwrap(), "from file\n");
        assert_eq!(spawn_task("words", &path).unwrap_err(), "give the task as text or with a task file, not both");
        let blank = scratch_file(" \n");
        let blank_path = blank.display().to_string();
        assert_eq!(spawn_task("", &blank_path).unwrap_err(), format!("task file {blank_path} is empty"));
        assert!(spawn_task("", "/nonexistent-task").unwrap_err().starts_with("task file: "));
        std::fs::remove_file(f).unwrap();
        std::fs::remove_file(blank).unwrap();
    }

    #[test]
    fn the_parent_is_this_terminal_unless_no_parent() {
        let p = parse("spawn", &words("--name b --profile builder do it")).unwrap();
        let s = spec_of(&p, "t3").unwrap();
        assert_eq!((s.parent.as_str(), s.task.as_str(), s.name.as_str()), ("t3", "do it", "b"));
        let p = parse("spawn", &words("--name b --no-parent")).unwrap();
        assert_eq!(spec_of(&p, "t3").unwrap().parent, "");
    }

    fn call(args: &str, config: Option<PathBuf>) -> (i32, String, String) {
        let env = Env { config_path: config, kit: None, home: None, terminal_id: String::new() };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&words(args), &env, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn flow_prints_the_flow_and_refuses_arguments() {
        let none = Some(PathBuf::from("/nonexistent/agents.json"));
        let (code, out, _) = call("flow", none.clone());
        assert_eq!((code, out), (0, Flow::default().text()));
        let (code, out, err) = call("flow now", none.clone());
        assert_eq!((code, out.as_str(), err.as_str()), (2, "", "contr-rune agents flow: takes no arguments\n"));
        let bad = scratch_file("{");
        let (code, _, err) = call("flow", Some(bad.clone()));
        assert_eq!(code, 1);
        assert!(err.starts_with("contr-rune agents: agents: "));
        // `help` falls back to the default flow when the file cannot be read.
        let (code, out, _) = call("help", Some(bad.clone()));
        assert_eq!(code, 0);
        assert!(out.starts_with("Usage:") && out.contains("The flow could not be read ("));
        std::fs::remove_file(bad).unwrap();
    }

    #[test]
    fn usage_errors_exit_2_and_later_parts_say_not_built_yet() {
        let (code, _, err) = call("", None);
        assert_eq!((code, err.starts_with("Usage:")), (2, true));
        let (code, _, err) = call("ls extra", None);
        assert_eq!(code, 2);
        assert!(err.starts_with("contr-rune agents ls: wrong number of arguments\n"));
        let (code, _, err) = call("bogus", None);
        assert_eq!(code, 2);
        assert!(err.starts_with("contr-rune agents: unknown command bogus\n"));
        let (code, _, err) = call("stop x", None);
        assert_eq!((code, err.as_str()), (2, "contr-rune agents stop: not built yet\n"));
    }
}
