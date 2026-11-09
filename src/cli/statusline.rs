//! `contr-rune statusline [-- <old command…>]`: Claude Code's status-line command (HC-PLAN H12, C8).
//! It reads the status-line JSON on stdin, keeps the plan's limit windows (`usage/claude.json`) and the
//! agent's context (`usage/context/<HC_TERMINAL_ID>.json`) in the state dir, then runs the old status line
//! (2 s limit) and prints its output. The readers are in `usage.rs`. Bad input and failed saves never
//! fail the call, so the status line keeps working.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::paths;
use crate::usage::{self, Context, Window};

/// The largest input that is used. A longer one is skipped whole: nothing is saved, the old command does not run.
const MAX_INPUT: u64 = 1024 * 1024;
/// How long the old command runs before it is killed.
const CHAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the output of a killed command is still collected.
const DRAIN: Duration = Duration::from_millis(100);

/// What a call depends on: the state dir, the agent's hc terminal, the clock and the chain limit.
pub struct Env {
    pub state: Option<PathBuf>,
    pub terminal: Option<String>,
    pub now: u64,
    pub timeout: Duration,
}

impl Env {
    pub fn real() -> Env {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let terminal = std::env::var("HC_TERMINAL_ID").ok().filter(|t| !t.is_empty());
        Env { state: paths::state_dir(), terminal, now, timeout: CHAIN_TIMEOUT }
    }
}

/// `args` follow the word `statusline`: nothing, or `--` and the old command with its arguments.
pub fn run(args: &[String], env: &Env, stdin: impl Read, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let then = match args.split_first() {
        None => &[][..],
        Some((first, rest)) if first == "--" => rest,
        Some(_) => {
            let _ = writeln!(err, "usage: contr-rune statusline [-- <old command> [args…]]");
            return 2;
        }
    };
    let mut input = Vec::new();
    // One byte past the cap tells an input that is too long from one of exactly the cap.
    if let Err(e) = stdin.take(MAX_INPUT + 1).read_to_end(&mut input) {
        let _ = writeln!(err, "contr-rune statusline: {e}");
        return 1;
    }
    if input.len() as u64 > MAX_INPUT {
        let _ = writeln!(err, "contr-rune statusline: input is over 1 MiB; skipped");
        return 0;
    }
    save(&input, env, err);
    if let Some((program, rest)) = then.split_first()
        && let Err(e) = out.write_all(&run_capped(program, rest, input, env.timeout))
    {
        let _ = writeln!(err, "contr-rune statusline: {e}");
        return 1;
    }
    0
}

fn save(input: &[u8], env: &Env, err: &mut impl Write) {
    let Ok(json) = serde_json::from_slice::<Value>(input) else { return };
    let Some(state) = &env.state else {
        let _ = writeln!(err, "contr-rune statusline: no state dir; usage not saved");
        return;
    };
    if let Some((version, windows)) = claude_windows(&json)
        && let Err(e) = write_claude(state, version.as_deref(), &windows, env.now)
    {
        let _ = writeln!(err, "contr-rune statusline: cannot write the Claude usage file: {e}");
    }
    if let (Some(id), Some(context)) = (&env.terminal, context(&json))
        && let Err(e) = write_context(state, id, &context, env.now)
    {
        let _ = writeln!(err, "contr-rune statusline: cannot write the context file: {e}");
    }
}

/// A limit window with its kind (`five_hour`, `seven_day`).
type Kinded = (&'static str, Window);

/// Claude's version and the limit windows that have a used percent; none when `rate_limits` is absent or
/// has no such window.
fn claude_windows(json: &Value) -> Option<(Option<String>, Vec<Kinded>)> {
    let limits = json.get("rate_limits")?;
    let windows: Vec<Kinded> = ["five_hour", "seven_day"]
        .into_iter()
        .filter_map(|kind| {
            let raw = limits.get(kind)?;
            let used_percent = raw.get("used_percentage")?.as_f64()?;
            Some((kind, Window { used_percent, resets_at: raw.get("resets_at").and_then(Value::as_u64) }))
        })
        .collect();
    let version = json.get("version").and_then(Value::as_str).map(str::to_owned);
    (!windows.is_empty()).then_some((version, windows))
}

/// The context window of the input; none without a window size or usage (a new session has `current_usage` null).
fn context(json: &Value) -> Option<Context> {
    let window = json.get("context_window")?;
    let size = window.get("context_window_size")?.as_u64().filter(|&s| s > 0)?;
    let usage = window.get("current_usage").filter(|u| u.is_object())?;
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let tokens = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    let model = json.get("model").map_or_else(String::new, |m| {
        let text = |key: &str| m.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
        short_model(&text("id"), &text("display_name"))
    });
    let effort = json.pointer("/effort/level").and_then(Value::as_str).unwrap_or("").to_owned();
    Some(Context { tokens, size, model, effort })
}

/// The family word of a model: `sonnet` for `claude-sonnet-5-5` or `Sonnet 5.5`; else the display name in
/// lower case, else the id.
fn short_model(id: &str, display: &str) -> String {
    let both = format!("{id} {display}").to_lowercase();
    if let Some(family) = ["opus", "sonnet", "haiku"].into_iter().find(|f| both.contains(f)) {
        family.to_owned()
    } else if display.is_empty() {
        id.to_owned()
    } else {
        display.to_lowercase()
    }
}

/// Writes `usage/claude.json` unless it already holds the same windows.
fn write_claude(state: &Path, version: Option<&str>, windows: &[Kinded], now: u64) -> std::io::Result<()> {
    let path = state.join("usage/claude.json");
    let plain: Vec<Window> = windows.iter().map(|(_, w)| w.clone()).collect();
    let old = std::fs::read_to_string(&path).ok().and_then(|t| usage::parse_claude(&t));
    if old.as_deref() == Some(&plain[..]) {
        return Ok(());
    }
    let windows: Vec<Value> = windows
        .iter()
        .map(|(kind, w)| json!({"kind": kind, "used_percent": w.used_percent, "resets_at": w.resets_at}))
        .collect();
    let file = json!({"schema": 1, "updated_at": now, "claude_version": version, "windows": windows});
    write_atomic(&path, &file)
}

/// Writes `usage/context/<id>.json` unless it already holds the same context.
fn write_context(state: &Path, id: &str, context: &Context, now: u64) -> std::io::Result<()> {
    let path = state.join("usage/context").join(format!("{}.json", paths::sanitize_id(id)));
    let old = std::fs::read_to_string(&path).ok().and_then(|t| usage::parse_context(&t));
    if old.as_ref() == Some(context) {
        return Ok(());
    }
    let file = json!({"schema": 1, "updated_at": now, "tokens": context.tokens, "size": context.size,
        "model": context.model, "effort": context.effort});
    write_atomic(&path, &file)
}

/// A temp file and a rename, so a reader never sees a half-written file.
fn write_atomic(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Runs `program args…` with `input` on its stdin and returns its stdout, whatever its exit status. A
/// command that runs past `timeout` is killed with the processes it forked and gives what it wrote so far.
/// A command that cannot start gives nothing.
fn run_capped(program: &str, args: &[String], input: Vec<u8>, timeout: Duration) -> Vec<u8> {
    let spawned = Command::new(program).args(args).process_group(0).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            eprintln!("contr-rune statusline: `{program}` did not start: {e}");
            return Vec::new();
        }
    };
    let pid = child.id();
    let (mut stdin, mut stdout) = (child.stdin.take(), child.stdout.take());
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&input);
        }
    });
    let collected = Arc::new(Mutex::new(Vec::new()));
    let (closed_tx, closed) = channel();
    let sink = Arc::clone(&collected);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Some(n) = stdout.as_mut().and_then(|o| o.read(&mut chunk).ok()).filter(|&n| n > 0) {
            sink.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
        let _ = closed_tx.send(());
    });
    let (exited_tx, exited) = channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = exited_tx.send(());
    });
    if exited.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout) {
        eprintln!("contr-rune statusline: `{program}` ran past its limit; killed");
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
    }
    // A process that escaped the kill may hold the pipe open, so the wait for the rest is short.
    let _ = closed.recv_timeout(DRAIN);
    collected.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("contr-rune-statusline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn env(state: &Path, terminal: Option<&str>) -> Env {
        Env { state: Some(state.to_owned()), terminal: terminal.map(str::to_owned), now: 1000, timeout: Duration::from_secs(2) }
    }

    fn call(args: &[&str], env: &Env, input: &[u8]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&args, env, input, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    fn read(dir: &Path, file: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap()
    }

    const FULL: &str = r#"{"version":"2.1.5","model":{"id":"claude-sonnet-5-5","display_name":"Sonnet 5.5"},
        "effort":{"level":"high"},
        "context_window":{"context_window_size":200000,"current_usage":{"input_tokens":10,"cache_creation_input_tokens":20000,"cache_read_input_tokens":70000}},
        "rate_limits":{"five_hour":{"used_percentage":38.0,"resets_at":2000},"seven_day":{"used_percentage":12.5}}}"#;

    #[test]
    fn windows_are_read_from_rate_limits() {
        let w = |used_percent, resets_at| Window { used_percent, resets_at };
        let cases = [
            (FULL, Some((Some("2.1.5".to_owned()), vec![("five_hour", w(38.0, Some(2000))), ("seven_day", w(12.5, None))]))),
            (r#"{"rate_limits":{"seven_day":{"used_percentage":5,"resets_at":9}}}"#, Some((None, vec![("seven_day", w(5.0, Some(9)))]))),
            // A window without a percent is skipped; no window at all gives nothing.
            (r#"{"rate_limits":{"five_hour":{"resets_at":9}}}"#, None),
            (r#"{"rate_limits":{}}"#, None),
            (r#"{"version":"1"}"#, None),
        ];
        for (text, want) in cases {
            assert_eq!(claude_windows(&serde_json::from_str(text).unwrap()), want, "{text}");
        }
    }

    #[test]
    fn context_is_the_summed_usage_with_a_short_model() {
        let ctx = |text: &str| context(&serde_json::from_str(text).unwrap());
        let got = ctx(FULL).unwrap();
        assert_eq!(got, Context { tokens: 90010, size: 200000, model: "sonnet".into(), effort: "high".into() });
        // No model or effort: both empty. A new session (null usage) or no size: nothing.
        let bare = r#"{"context_window":{"context_window_size":1000000,"current_usage":{"input_tokens":5}}}"#;
        assert_eq!(ctx(bare), Some(Context { tokens: 5, size: 1_000_000, model: String::new(), effort: String::new() }));
        assert_eq!(ctx(r#"{"context_window":{"context_window_size":200000,"current_usage":null}}"#), None);
        assert_eq!(ctx(r#"{"context_window":{"context_window_size":0,"current_usage":{}}}"#), None);
        assert_eq!(ctx("{}"), None);
    }

    #[test]
    fn short_model_names_the_family() {
        let cases = [
            ("claude-opus-4-1", "", "opus"),
            ("x", "Sonnet 5.5", "sonnet"),
            ("claude-haiku-4", "Haiku", "haiku"),
            ("some-id", "Fancy Model", "fancy model"),
            ("some-id", "", "some-id"),
        ];
        for (id, display, want) in cases {
            assert_eq!(short_model(id, display), want);
        }
    }

    #[test]
    fn the_files_are_read_back_by_the_usage_readers() {
        let dir = scratch("roundtrip");
        let (code, out, err) = call(&[], &env(&dir, Some("t/3")), FULL.as_bytes());
        assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
        let claude = std::fs::read_to_string(dir.join("usage/claude.json")).unwrap();
        let windows = usage::parse_claude(&claude).unwrap();
        assert_eq!(windows, vec![Window { used_percent: 38.0, resets_at: Some(2000) }, Window { used_percent: 12.5, resets_at: None }]);
        assert_eq!(usage::percent_used(&windows, 1000), Some(38));
        let v = read(&dir, "usage/claude.json");
        assert_eq!((&v["updated_at"], &v["claude_version"], &v["windows"][0]["kind"]), (&json!(1000), &json!("2.1.5"), &json!("five_hour")));
        let text = std::fs::read_to_string(dir.join("usage/context/t_3.json")).unwrap();
        let context = usage::parse_context(&text).unwrap();
        assert_eq!(context.label(), "90k of 200k  45%");
        assert_eq!((context.model.as_str(), context.effort.as_str()), ("sonnet", "high"));
    }

    #[test]
    fn unchanged_windows_and_context_are_not_written_again() {
        let dir = scratch("same");
        call(&[], &env(&dir, Some("t1")), FULL.as_bytes());
        let later = Env { now: 5000, ..env(&dir, Some("t1")) };
        call(&[], &later, FULL.as_bytes());
        for file in ["usage/claude.json", "usage/context/t1.json"] {
            assert_eq!(read(&dir, file)["updated_at"], json!(1000), "{file}");
        }
        // A changed window is written.
        call(&[], &later, FULL.replace("38.0", "40.0").as_bytes());
        let v = read(&dir, "usage/claude.json");
        assert_eq!((&v["updated_at"], &v["windows"][0]["used_percent"]), (&json!(5000), &json!(40.0)));
    }

    #[test]
    fn no_terminal_id_means_no_context_file_and_bad_input_writes_nothing() {
        let dir = scratch("none");
        call(&[], &env(&dir, None), FULL.as_bytes());
        assert!(dir.join("usage/claude.json").exists());
        assert!(!dir.join("usage/context").exists());
        let other = scratch("bad");
        for input in ["not json", "{}", ""] {
            assert_eq!(call(&[], &env(&other, Some("t1")), input.as_bytes()).0, 0);
        }
        assert!(!other.join("usage").exists());
    }

    #[test]
    fn input_over_1_mib_is_skipped_whole() {
        let dir = scratch("big");
        let big = format!("{FULL}{}", " ".repeat(MAX_INPUT as usize));
        let marker = dir.join("ran");
        let script = format!("touch {}", marker.display());
        let (code, out, _) = call(&["--", "sh", "-c", &script], &env(&dir, Some("t1")), big.as_bytes());
        assert_eq!((code, out.as_str()), (0, ""));
        assert!(!dir.join("usage").exists());
        assert!(!marker.exists());
    }

    #[test]
    fn the_old_command_gets_the_input_and_its_output_is_printed() {
        let dir = scratch("chain");
        let (code, out, _) = call(&["--", "sh", "-c", "head -c 20; echo; exit 3"], &env(&dir, None), FULL.as_bytes());
        assert_eq!(code, 0);
        assert_eq!(out, format!("{}\n", &FULL[..20]));
        // A command that cannot start prints nothing; the files are still written.
        let (code, out, _) = call(&["--", "/no/such/command"], &env(&dir, None), FULL.as_bytes());
        assert_eq!((code, out.as_str()), (0, ""));
        assert!(dir.join("usage/claude.json").exists());
    }

    #[test]
    fn a_slow_old_command_is_killed_and_gives_what_it_wrote() {
        let dir = scratch("slow");
        let fast = Env { timeout: Duration::from_millis(300), ..env(&dir, None) };
        let started = std::time::Instant::now();
        let (code, out, _) = call(&["--", "sh", "-c", "echo early; sleep 5; echo late"], &fast, FULL.as_bytes());
        assert_eq!((code, out.as_str()), (0, "early\n"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn arguments_other_than_a_chain_are_a_usage_error() {
        let dir = scratch("usage");
        let (code, out, err) = call(&["--bogus"], &env(&dir, None), b"{}");
        assert_eq!((code, out.as_str()), (2, ""));
        assert!(err.starts_with("usage: contr-rune statusline"));
    }
}
