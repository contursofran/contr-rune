//! `contr-rune statusline -- <old command>` as a child process with a scratch `CONTR_STATE_DIR`. The files
//! are read back by the app's own readers in the unit tests (`src/cli/statusline.rs`); here the process
//! contract is checked: files written, old output chained, the 2 s limit, the 1 MiB rule.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const INPUT: &str = r#"{"version":"2.1.5","model":{"id":"claude-opus-4-1","display_name":"Opus"},
    "context_window":{"context_window_size":200000,"current_usage":{"input_tokens":1000}},
    "rate_limits":{"five_hour":{"used_percentage":21.0,"resets_at":4102444800}}}"#;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("contr-rune-statusline-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Pipes `input` into `contr-rune statusline -- sh -c <script>`; returns stdout and how long it took.
fn run(state: &Path, script: &str, input: &[u8]) -> (String, Duration) {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .args(["statusline", "--", "sh", "-c", script])
        .env("CONTR_STATE_DIR", state)
        .env("HC_TERMINAL_ID", "t9")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(out.status.success());
    (String::from_utf8(out.stdout).unwrap(), started.elapsed())
}

#[test]
fn files_are_written_and_the_old_status_line_output_is_chained() {
    let state = scratch("chain");
    let (out, _) = run(&state, "cat >/dev/null; echo old-line", INPUT.as_bytes());
    assert_eq!(out, "old-line\n");
    let claude: Value = serde_json::from_str(&std::fs::read_to_string(state.join("usage/claude.json")).unwrap()).unwrap();
    assert_eq!(claude["windows"][0]["used_percent"], 21.0);
    let context: Value = serde_json::from_str(&std::fs::read_to_string(state.join("usage/context/t9.json")).unwrap()).unwrap();
    assert_eq!((&context["tokens"], &context["size"], &context["model"]), (&1000.into(), &200000.into(), &"opus".into()));
}

#[test]
fn an_old_command_past_2_seconds_is_killed_and_its_early_output_kept() {
    let state = scratch("slow");
    let (out, took) = run(&state, "echo early; sleep 10", INPUT.as_bytes());
    assert_eq!(out, "early\n");
    assert!(took < Duration::from_secs(4), "took {took:?}");
    assert!(state.join("usage/claude.json").exists());
}

#[test]
fn input_over_1_mib_saves_nothing_and_does_not_run_the_old_command() {
    let state = scratch("big");
    let marker = state.join("ran");
    let big = format!("{INPUT}{}", " ".repeat(1024 * 1024));
    let (out, _) = run(&state, &format!("touch {}; echo old-line", marker.display()), big.as_bytes());
    assert_eq!(out, "");
    assert!(!marker.exists());
    assert!(!state.join("usage").exists());
}
