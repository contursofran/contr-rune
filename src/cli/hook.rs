//! `contr-rune hook stop` and `hook session-start` (PARITY-PLAN D5, D6; Go: cmd/contr/hook.go).
//! Claude Code gives the hook input as one JSON value on stdin. Stop, in a worker
//! (`CONTR_COORDINATOR` set): a turn that sent no report gets one sent to the coordinator; in any
//! hc session the terminal gets the `contr.model` and `contr.effort` labels (D189) and, outside a
//! worker, `contr.context`, `contr.context-limit` and `contr.orchestrating` (D191); in a
//! coordinator past `CONTR_SAVE_AT` tokens the turn is kept once to save `COORDINATOR-STATE.md`.
//! session-start, after a compaction in a coordinator, prints what it needs to carry on. It never
//! exits 2: for a Stop hook that blocks the turn.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use super::{hook_session, restore, turn};
use crate::agents;
use crate::hc::conn::Conn;
use crate::hc::owner::OwnerFile;

const USAGE: &str = "\
Usage:
  contr-rune hook stop            the kit's Stop hook
  contr-rune hook session-start   the kit's SessionStart hook (matcher compact)
  contr-rune hook post-tool       the kit's PostToolUse hook: saves the todos and artifacts
  contr-rune hook prompt          the kit's UserPromptSubmit hook: asks for [you] todos

Claude Code runs it and gives it the hook input on stdin.

stop, in a worker (CONTR_COORDINATOR is set): a turn that sent no report with hc send
gets one, \"ended its turn without a report. Last words: ...\", sent to the coordinator.
In any hc session it also labels the terminal with the model and effort (contr.model,
contr.effort) for the sidebar row. It never stops the turn.

stop, in a coordinator (a session that ran /contr:orchestrate, or an hc terminal with role
orchestrator): once the context passes CONTR_SAVE_AT tokens (default 220000), it keeps the turn
going one time and asks the coordinator to update COORDINATOR-STATE.md before Claude Code
compacts.

session-start, after a compaction in a coordinator: prints the playbook, the state file to
read, the live workers it started, its open owner items and the flow (contr-rune agents flow),
which Claude Code adds to the new context.

post-tool, in an hc terminal: TodoWrite, TaskCreate, TaskUpdate and claude-artifacts calls are
saved in the session file (<state>/session/<id>.json) that the app shows. It always exits 0.

prompt, in an hc terminal that is not a worker: asks Claude to add each owner request to its
todo list with the text starting \"[you] \". Prompts from hc (<hc from=) are skipped. It always
exits 0.
";

/// The most characters of the worker's last words in a report.
const SAID_MAX: usize = 300;

/// How long to wait for the hook input, and for each call to the core.
const INPUT_WAIT: Duration = Duration::from_secs(5);
const CALL: Duration = Duration::from_secs(10);

/// The environment a hook reads.
pub struct Env {
    /// `CONTR_COORDINATOR`: the address that started this worker; empty outside a worker.
    pub coordinator: String,
    /// `HC_TERMINAL_ID`: this terminal; empty outside hc.
    pub terminal_id: String,
    /// `CLAUDE_EFFORT`.
    pub effort: String,
    /// `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, as written.
    pub compact_window: String,
    pub restore: restore::Env,
}

impl Env {
    pub fn real() -> Env {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        Env { coordinator: var("CONTR_COORDINATOR"), terminal_id: var("HC_TERMINAL_ID"), effort: var("CLAUDE_EFFORT"), compact_window: var("CLAUDE_CODE_AUTO_COMPACT_WINDOW"), restore: restore::Env::real() }
    }
}

/// `contr-rune hook <kind>`. Exit 0 when done or nothing to do, 1 on an error (usage included);
/// never 2.
pub fn run(args: &[String], env: &Env, stdin: impl Read + Send + 'static, out: &mut impl Write, err: &mut impl Write) -> i32 {
    match args {
        [kind] if kind == "stop" => stop(env, stdin, out, err),
        [kind] if kind == "session-start" => session_start(env, stdin, out, err),
        [kind] if ["post-tool", "prompt"].contains(&kind.as_str()) => hook_session::run(kind, env, stdin, out, err),
        _ => {
            let _ = err.write_all(USAGE.as_bytes());
            1
        }
    }
}

fn stop(env: &Env, stdin: impl Read + Send + 'static, out: &mut impl Write, err: &mut impl Write) -> i32 {
    // Outside hc there are no workers and no sessions to label.
    if env.coordinator.is_empty() && env.terminal_id.is_empty() {
        return 0;
    }
    let input = match read_input(stdin) {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(err, "contr-rune hook: bad hook input: {e}");
            return 1;
        }
    };
    let transcript = input["transcript_path"].as_str().unwrap_or_default();
    // The report is the work; the labels are best effort and never fail the hook.
    let report = if env.coordinator.is_empty() { Ok(None) } else { missed_turn(&input, transcript) };
    let labels = labels(&input, transcript, env);
    // A coordinator past the save size: the tokens, else nothing to ask.
    let ask = if env.coordinator.is_empty() { save_request(&input, transcript, &env.restore) } else { Ok(None) };
    if report.as_ref().is_ok_and(Option::is_none) && labels.is_empty() && ask.as_ref().is_ok_and(Option::is_none) {
        return 0;
    }
    let (conn, _keep) = match super::agents::connect() {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "contr-rune hook: {e}");
            return 1;
        }
    };
    if !labels.is_empty() {
        label(&conn, &env.terminal_id, &labels);
    }
    let mut code = 0;
    let mut fail = |e: String| {
        let _ = writeln!(err, "contr-rune hook: {e}");
        code = 1;
    };
    match report {
        Ok(Some(text)) => {
            if let Err(e) = agents::send(&conn, &env.terminal_id, &env.coordinator, &text) {
                fail(format!("report to {}: {e}", env.coordinator));
            }
        }
        Ok(None) => {}
        Err(e) => fail(e),
    }
    match ask {
        Ok(Some(tokens)) => {
            if let Err(e) = ask_to_save(&conn, &input, tokens, env, out) {
                fail(e);
            }
        }
        Ok(None) => {}
        Err(e) => fail(e),
    }
    code
}

/// The context size when this Stop may ask for the state file: not inside a hook-kept turn, the
/// size is at `CONTR_SAVE_AT` or more, and the session was not asked before.
fn save_request(input: &Value, transcript: &str, env: &restore::Env) -> Result<Option<u64>, String> {
    if input["stop_hook_active"] == true || transcript.is_empty() {
        return Ok(None);
    }
    let tokens = turn::context_tokens(Path::new(transcript)).map_err(|e| format!("transcript {transcript:?}: {e}"))?;
    let asked = env.asked(input["session_id"].as_str().unwrap_or_default());
    Ok(restore::should_ask(tokens, env.save_at(), asked).then_some(tokens))
}

/// Keeps the turn going once when the session is a coordinator: marks it asked, then prints the
/// block decision.
fn ask_to_save(conn: &Conn, input: &Value, tokens: u64, env: &Env, out: &mut impl Write) -> Result<(), String> {
    if !is_coordinator(conn, input, &env.terminal_id)? {
        return Ok(());
    }
    env.restore.mark_asked(input["session_id"].as_str().unwrap_or_default())?;
    let decision = json!({"decision": "block", "reason": restore::ask_reason(tokens)});
    writeln!(out, "{decision}").map_err(|e| e.to_string())
}

/// Whether the session is a coordinator: it ran `/contr:orchestrate`, or its hc terminal has the
/// role orchestrator.
fn is_coordinator(conn: &Conn, input: &Value, terminal: &str) -> Result<bool, String> {
    if let Some(transcript) = input["transcript_path"].as_str().filter(|t| !t.is_empty()) {
        match turn::ran_command(Path::new(transcript), restore::ORCHESTRATE_COMMAND) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("transcript {transcript:?}: {e}")),
        }
    }
    if terminal.is_empty() {
        return Ok(false);
    }
    Ok(agents::get(conn, "", terminal)?.role == "orchestrator")
}

/// `hook session-start`: after a compaction (source `compact`, or none) in a coordinator, prints
/// the restore text, which Claude Code adds to the new context. Any other source prints nothing.
fn session_start(env: &Env, stdin: impl Read + Send + 'static, out: &mut impl Write, err: &mut impl Write) -> i32 {
    if env.coordinator.is_empty() && env.terminal_id.is_empty() {
        return 0;
    }
    let input = match read_input(stdin) {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(err, "contr-rune hook: bad hook input: {e}");
            return 1;
        }
    };
    if input["source"].as_str().is_some_and(|s| !s.is_empty() && s != "compact") {
        return 0;
    }
    env.restore.clear_asked(input["session_id"].as_str().unwrap_or_default());
    let conn = super::agents::connect();
    let result = match &conn {
        Ok((conn, _keep)) => is_coordinator(conn, &input, &env.terminal_id),
        Err(e) => Err(e.clone()),
    };
    match result {
        Ok(true) => {}
        Ok(false) => return 0,
        Err(e) => {
            let _ = writeln!(err, "contr-rune hook: {e}");
            return 1;
        }
    }
    let workers = conn.as_ref().map_err(Clone::clone).and_then(|(c, _)| agents::list(c, false));
    let owner = match &env.restore.owner_file {
        Some(path) => OwnerFile::new(path.clone()).open_items(None).map_err(|e| e.to_string()),
        None => Err("no home folder to find owner.json in".to_owned()),
    };
    let r = restore::Restore {
        cwd: input["cwd"].as_str().unwrap_or_default(),
        me: &env.terminal_id,
        workers,
        owner,
        flow: super::agents::flow_text(env.restore.config.as_deref()),
        playbook: restore::playbook(env.restore.kit.as_deref()),
    };
    match write!(out, "{}", restore::restore_text(&r)) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "contr-rune hook: {e}");
            1
        }
    }
}

/// One JSON value from `stdin`, within `INPUT_WAIT`. It reads one value, not to the end: a
/// caller that keeps the pipe open must not hang the hook (Swift P21).
pub(super) fn read_input(stdin: impl Read + Send + 'static) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let value = serde_json::Deserializer::from_reader(stdin).into_iter::<Value>().next();
        let _ = tx.send(value);
    });
    match rx.recv_timeout(INPUT_WAIT) {
        Ok(Some(Ok(v))) => Ok(v),
        Ok(Some(Err(e))) => Err(e.to_string()),
        Ok(None) => Err("empty input".into()),
        Err(_) => Err("no input within 5 s".into()),
    }
}

/// The text to send the coordinator when the last turn sent no report, else `None`.
fn missed_turn(input: &Value, transcript: &str) -> Result<Option<String>, String> {
    let bytes = std::fs::read(transcript).map_err(|e| format!("transcript {transcript:?}: {e}"))?;
    let turn = turn::last_turn(&bytes);
    if turn.reported {
        return Ok(None);
    }
    let said = input["last_assistant_message"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(&turn.said);
    Ok(Some(missed_report(said, input["cwd"].as_str().unwrap_or_default())))
}

/// The report for a turn that sent none. `said` is cut to one line of `SAID_MAX` characters;
/// `RESULT.md` in the worker's folder is pointed to.
fn missed_report(said: &str, cwd: &str) -> String {
    let mut said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.chars().count() > SAID_MAX {
        said = said.chars().take(SAID_MAX).collect::<String>() + "…";
    }
    if said.is_empty() {
        said = "(none)".into();
    }
    let mut text = format!("ended its turn without a report. Last words: {said}");
    if !cwd.is_empty() && Path::new(cwd).join("RESULT.md").exists() {
        text.push_str(&format!("; RESULT.md in {cwd}"));
    }
    text
}

/// The labels to set: the short model of the transcript and the effort of the hook input, else
/// `CLAUDE_EFFORT`. A value that is not known is left out, so the label keeps its last value.
/// Outside a worker also the context size and `contr.orchestrating` (Go `coordinatorStop`).
fn labels(input: &Value, transcript: &str, env: &Env) -> Vec<(&'static str, String)> {
    let mut labels = Vec::new();
    if env.terminal_id.is_empty() {
        return labels;
    }
    if !transcript.is_empty()
        && let Some(model) = turn::last_model(Path::new(transcript)).ok().flatten()
    {
        labels.push(("contr.model", turn::short_model(&model)));
    }
    let effort = input["effort"]["level"].as_str().filter(|l| !l.is_empty()).unwrap_or(&env.effort);
    if !effort.is_empty() {
        labels.push(("contr.effort", effort.to_owned()));
    }
    if env.coordinator.is_empty() && !transcript.is_empty() {
        let path = Path::new(transcript);
        labels.extend(context_labels(turn::context_tokens(path).unwrap_or(0), &env.compact_window));
        if turn::ran_command(path, restore::ORCHESTRATE_COMMAND).unwrap_or(false) {
            labels.push(("contr.orchestrating", "1".to_owned()));
        }
    }
    labels
}

/// The compaction window the context is shown against when `CLAUDE_CODE_AUTO_COMPACT_WINDOW` is
/// not a positive number.
const DEFAULT_COMPACT_WINDOW: u64 = 250_000;

/// `contr.context` (tokens) and `contr.context-limit` (the compaction window). 0 tokens gives an
/// empty `contr.context`, which removes the label, and no limit.
fn context_labels(tokens: u64, window: &str) -> Vec<(&'static str, String)> {
    if tokens == 0 {
        return vec![("contr.context", String::new())];
    }
    let limit = window.trim().parse::<u64>().ok().filter(|&n| n > 0).unwrap_or(DEFAULT_COMPACT_WINDOW);
    vec![("contr.context", tokens.to_string()), ("contr.context-limit", limit.to_string())]
}

/// `terminal.label` on this terminal. Best effort.
fn label(conn: &Conn, terminal: &str, labels: &[(&str, String)]) {
    let map: serde_json::Map<String, Value> = labels.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect();
    let _ = conn.call("terminal.label", json!({"terminal": terminal, "labels": map}), CALL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_report_table() {
        let long = "x".repeat(400);
        let cut = format!("ended its turn without a report. Last words: {}…", "x".repeat(300));
        let cases = [
            ("done  the\n thing", "", "ended its turn without a report. Last words: done the thing"),
            ("", "", "ended its turn without a report. Last words: (none)"),
            (" \n ", "/nonexistent-dir", "ended its turn without a report. Last words: (none)"),
            (long.as_str(), "", cut.as_str()),
        ];
        for (said, cwd, want) in cases {
            assert_eq!(missed_report(said, cwd), want);
        }
    }

    #[test]
    fn missed_report_points_to_result_md() {
        let dir = std::env::temp_dir().join(format!("contr-rune-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("RESULT.md"), "x").unwrap();
        let cwd = dir.to_str().unwrap();
        assert_eq!(missed_report("hi", cwd), format!("ended its turn without a report. Last words: hi; RESULT.md in {cwd}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_take_the_effort_of_the_input_before_the_environment() {
        let env = |effort: &str, id: &str| Env { coordinator: String::new(), terminal_id: id.into(), effort: effort.into(), compact_window: String::new(), restore: restore::Env::default() };
        let input = json!({"effort": {"level": "high"}});
        assert_eq!(labels(&input, "", &env("low", "t1")), vec![("contr.effort", "high".to_owned())]);
        assert_eq!(labels(&json!({}), "", &env("low", "t1")), vec![("contr.effort", "low".to_owned())]);
        assert!(labels(&json!({}), "", &env("", "t1")).is_empty());
        assert!(labels(&input, "", &env("low", "")).is_empty());
    }

    #[test]
    fn context_labels_table() {
        let l = |t: &str, w: &str| vec![("contr.context", t.to_owned()), ("contr.context-limit", w.to_owned())];
        assert_eq!(context_labels(0, "100000"), vec![("contr.context", String::new())]);
        assert_eq!(context_labels(90_000, ""), l("90000", "250000"));
        assert_eq!(context_labels(90_000, " 1000000 "), l("90000", "1000000"));
        for bad in ["0", "-1", "big"] {
            assert_eq!(context_labels(5, bad), l("5", "250000"), "{bad}");
        }
    }

    #[test]
    fn input_is_one_json_value_and_bad_input_exits_1_never_2() {
        assert_eq!(read_input(&b"{\"a\":1} trailing"[..]).unwrap(), json!({"a": 1}));
        assert!(read_input(&b""[..]).is_err());
        let env = Env { coordinator: "t3".into(), terminal_id: "t1".into(), effort: String::new(), compact_window: String::new(), restore: restore::Env::default() };
        let mut err = Vec::new();
        assert_eq!(run(&["stop".into()], &env, &b"nope"[..], &mut Vec::new(), &mut err), 1);
        assert!(String::from_utf8(err).unwrap().contains("bad hook input"));
        let outside = Env { coordinator: String::new(), terminal_id: String::new(), effort: String::new(), compact_window: String::new(), restore: restore::Env::default() };
        assert_eq!(run(&["stop".into()], &outside, &b""[..], &mut Vec::new(), &mut Vec::new()), 0);
        for args in [vec![], vec!["bogus".to_owned()], vec!["prompt".to_owned(), "extra".to_owned()]] {
            assert_eq!(run(&args, &env, &b""[..], &mut Vec::new(), &mut Vec::new()), 1);
        }
    }
}
