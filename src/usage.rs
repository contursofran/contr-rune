//! Plan usage and agent context, read from the files Go contr writes (PARITY-PLAN K2, HC-PLAN C8).
//!
//! * Claude: `<state>/usage/claude.json`, written by the status line.
//! * Codex: the newest `rollout-*.jsonl` under `$CODEX_HOME/sessions` (read only, last 256 KiB).
//! * Context: `<state>/usage/context/<id>.json` (read by the context meter, H13).
//!
//! Parsing is pure and tested. File reads run on a thread (`Plans`), at most once a minute.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rune::{Tx, Waker};
use serde::Deserialize;
use serde_json::Value;

use crate::paths;

/// How often the plan files are read while the window has the focus.
const EVERY: Duration = Duration::from_secs(60);
/// How much of the end of a Codex rollout is read.
const TAIL: u64 = 256 * 1024;
/// The newest day folders of the Codex sessions that are searched.
const CODEX_DAYS: usize = 2;

/// One rate-limit window of a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub used_percent: f64,
    pub resets_at: Option<u64>,
}

/// The percent of a plan used: the highest of its live windows. A window past its reset counts as 0;
/// no window means no data.
pub fn percent_used(windows: &[Window], now: u64) -> Option<u32> {
    let used = windows.iter().map(|w| if w.resets_at.is_some_and(|r| r <= now) { 0.0 } else { w.used_percent });
    used.reduce(f64::max).map(|p| p.clamp(0.0, 100.0).round() as u32)
}

#[derive(Deserialize)]
struct ClaudeFile {
    schema: u32,
    #[serde(default)]
    windows: Vec<ClaudeWindow>,
}

#[derive(Deserialize)]
struct ClaudeWindow {
    used_percent: f64,
    resets_at: Option<u64>,
}

/// The windows of `usage/claude.json`; none for a bad file or another schema.
pub fn parse_claude(text: &str) -> Option<Vec<Window>> {
    let file: ClaudeFile = serde_json::from_str(text).ok()?;
    if file.schema != 1 {
        return None;
    }
    Some(file.windows.into_iter().map(|w| Window { used_percent: w.used_percent, resets_at: w.resets_at }).collect())
}

/// The windows of the last usable `token_count` line of a Codex rollout. `mtime` (Unix seconds) turns a
/// relative `resets_in_seconds` into a reset time. The first line is skipped when `partial` (the read
/// started in the middle of the file).
pub fn parse_codex(text: &str, mtime: u64, partial: bool) -> Option<Vec<Window>> {
    text.lines().skip(usize::from(partial)).filter_map(|line| codex_line(line, mtime)).last()
}

fn codex_line(line: &str, mtime: u64) -> Option<Vec<Window>> {
    if !line.contains("token_count") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let payload = v.get("payload").unwrap_or(&v);
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let limits = payload.get("rate_limits")?;
    if limits.get("limit_id").and_then(Value::as_str).is_some_and(|id| id != "codex") {
        return None;
    }
    // The primary window must have a percent; the secondary one is optional.
    let primary = codex_window(limits.get("primary")?, mtime)?;
    let mut windows = vec![primary];
    if let Some(w) = limits.get("secondary").and_then(|s| codex_window(s, mtime)) {
        windows.push(w);
    }
    Some(windows)
}

fn codex_window(v: &Value, mtime: u64) -> Option<Window> {
    let used_percent = v.get("used_percent").and_then(Value::as_f64)?;
    let resets_at = v
        .get("resets_at")
        .and_then(Value::as_u64)
        .or_else(|| v.get("resets_in_seconds").and_then(Value::as_u64).map(|s| mtime + s));
    Some(Window { used_percent, resets_at })
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn mtime_secs(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// The percent of the Claude plan used, from the state dir's file.
pub fn read_claude(state: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(state.join("usage/claude.json")).ok()?;
    percent_used(&parse_claude(&text)?, unix_now())
}

/// `$CODEX_HOME`, else `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(PathBuf::from(std::env::var_os("HOME")?).join(".codex")),
    }
}

/// Subfolders of `dir`, newest (largest name) first. Day folders are `YYYY/MM/DD`, so a text sort is a date sort.
fn dirs_desc(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by(|a, b| b.cmp(a));
    dirs
}

/// The newest rollout file (by modification time) of the newest day folders.
fn newest_rollout(sessions: &Path) -> Option<PathBuf> {
    let days = dirs_desc(sessions)
        .into_iter()
        .flat_map(|y| dirs_desc(&y))
        .flat_map(|m| dirs_desc(&m))
        .take(CODEX_DAYS);
    let mut best: Option<(u64, PathBuf)> = None;
    for day in days {
        let Ok(entries) = std::fs::read_dir(day) else { continue };
        for e in entries.flatten() {
            let (path, name) = (e.path(), e.file_name());
            let name = name.to_string_lossy();
            if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                continue;
            }
            if let Some(t) = mtime_secs(&path)
                && best.as_ref().is_none_or(|(b, _)| t > *b)
            {
                best = Some((t, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// The percent of the Codex plan used, from the newest rollout under `home`.
pub fn read_codex(home: &Path) -> Option<u32> {
    let path = newest_rollout(&home.join("sessions"))?;
    let mtime = mtime_secs(&path)?;
    let mut file = File::open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    percent_used(&parse_codex(&String::from_utf8_lossy(&bytes), mtime, start > 0)?, unix_now())
}

/// The plan meters' values: percent used, `None` for no data.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub claude: Option<u32>,
    pub codex: Option<u32>,
}

fn read_all() -> Reading {
    Reading {
        claude: paths::state_dir().and_then(|d| read_claude(&d)),
        codex: codex_home().and_then(|h| read_codex(&h)),
    }
}

/// The plan meters: the last reading, and the thread that refreshes it every minute while focused.
pub struct Plans {
    pub reading: Reading,
    last: Option<Instant>,
    inflight: bool,
    tx: Sender<Reading>,
    rx: Receiver<Reading>,
}

impl Plans {
    pub fn new() -> Plans {
        let (tx, rx) = channel();
        Plans { reading: Reading::default(), last: None, inflight: false, tx, rx }
    }

    /// Read now, on this thread (a headless run cannot wait for the thread). A headless run reads only
    /// the places `CONTR_STATE_DIR` and `CODEX_HOME` name, never the owner's real files.
    pub fn read_now(&mut self) {
        let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let all = read_all();
        self.reading = Reading { claude: all.claude.filter(|_| set("CONTR_STATE_DIR")), codex: all.codex.filter(|_| set("CODEX_HOME")) };
        self.last = Some(Instant::now());
    }

    /// Take a finished reading and start the next one when a minute has passed. Nothing starts while the
    /// window has no focus. Returns whether the reading changed.
    pub fn poll(&mut self, focused: bool, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(r) = self.rx.try_recv() {
            self.inflight = false;
            changed |= r != self.reading;
            self.reading = r;
        }
        if !focused {
            return changed;
        }
        if !self.inflight && self.last.is_none_or(|t| t.elapsed() >= EVERY) {
            self.inflight = true;
            self.last = Some(Instant::now());
            let (out, waker) = (self.tx.clone(), waker.cloned());
            std::thread::spawn(move || {
                let _ = out.send(read_all());
                if let Some(w) = &waker {
                    w();
                }
            });
        }
        tx.wake_after(EVERY);
        changed
    }
}

/// The context window of one agent (`usage/context/<id>.json`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Context {
    pub tokens: u64,
    pub size: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
}

#[allow(dead_code)]
impl Context {
    /// Whole percent of the window used, 0 to 100.
    pub fn percent(&self) -> u64 {
        (self.tokens * 100).checked_div(self.size).map_or(0, |p| p.min(100))
    }

    /// The meter's text: `90k of 200k  45%`.
    pub fn label(&self) -> String {
        format!("{} of {}  {}%", short_tokens(self.tokens), short_tokens(self.size), self.percent())
    }
}

/// `90k`; `1M` when `n` is a whole number of millions.
#[allow(dead_code)]
fn short_tokens(n: u64) -> String {
    if n >= 1_000_000 && n.is_multiple_of(1_000_000) {
        format!("{}M", n / 1_000_000)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// The context in a file's text; none for a bad file, another schema or no window size.
pub fn parse_context(text: &str) -> Option<Context> {
    #[derive(Deserialize)]
    struct Raw {
        schema: u32,
        #[serde(flatten)]
        context: Context,
    }
    let raw: Raw = serde_json::from_str(text).ok()?;
    (raw.schema == 1 && raw.context.size > 0).then_some(raw.context)
}

/// The context saved for hc terminal `id`.
#[allow(dead_code)]
pub fn read_context(id: &str) -> Option<Context> {
    parse_context(&std::fs::read_to_string(paths::context_file(id)?).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(used_percent: f64, resets_at: Option<u64>) -> Window {
        Window { used_percent, resets_at }
    }

    #[test]
    fn percent_is_the_highest_live_window() {
        let cases: [(&[Window], Option<u32>); 6] = [
            (&[], None),
            (&[w(38.0, None)], Some(38)),
            (&[w(38.4, Some(200)), w(61.6, Some(200))], Some(62)),
            // A window past its reset counts as 0.
            (&[w(90.0, Some(100)), w(12.0, Some(500))], Some(12)),
            (&[w(90.0, Some(99))], Some(0)),
            (&[w(140.0, None)], Some(100)),
        ];
        for (windows, want) in cases {
            assert_eq!(percent_used(windows, 100), want, "{windows:?}");
        }
    }

    #[test]
    fn claude_file_variants() {
        let ok = r#"{"schema":1,"updated_at":5,"claude_version":null,"windows":[
            {"kind":"five_hour","used_percent":38.0,"resets_at":900},{"kind":"seven_day","used_percent":7.5,"resets_at":null}]}"#;
        assert_eq!(parse_claude(ok), Some(vec![w(38.0, Some(900)), w(7.5, None)]));
        assert_eq!(parse_claude(r#"{"schema":1}"#), Some(vec![]));
        assert_eq!(parse_claude(r#"{"schema":2,"windows":[]}"#), None);
        assert_eq!(parse_claude("not json"), None);
        assert_eq!(parse_claude(""), None);
    }

    fn token_count(limit_id: Option<&str>, primary: &str, secondary: &str) -> String {
        let id = limit_id.map_or(String::new(), |i| format!(r#""limit_id":"{i}","#));
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{{id}"primary":{primary},"secondary":{secondary}}}}}}}"#
        )
    }

    #[test]
    fn codex_rollout_variants() {
        let p1 = r#"{"used_percent":12.0,"resets_at":2000}"#;
        let p2 = r#"{"used_percent":40.0,"resets_in_seconds":60}"#;
        let s = r#"{"used_percent":3.0,"resets_at":9000}"#;
        // The last usable line wins; a missing limit_id or "codex" counts, another id does not.
        let text = [token_count(Some("codex"), p1, s), token_count(Some("other"), p2, s), "{\"type\":\"event_msg\"}".to_owned()].join("\n");
        assert_eq!(parse_codex(&text, 1000, false), Some(vec![w(12.0, Some(2000)), w(3.0, Some(9000))]));
        // resets_in_seconds is relative to the file's mtime.
        let text = [token_count(Some("codex"), p1, s), token_count(None, p2, s)].join("\n");
        assert_eq!(parse_codex(&text, 1000, false), Some(vec![w(40.0, Some(1060)), w(3.0, Some(9000))]));
        // A primary window without a percent is not usable; the earlier line is.
        let text = [token_count(None, p1, "null"), token_count(None, r#"{"resets_at":1}"#, s)].join("\n");
        assert_eq!(parse_codex(&text, 1000, false), Some(vec![w(12.0, Some(2000))]));
        // A read that starts mid-file drops its first (cut) line.
        assert_eq!(parse_codex(&token_count(None, p1, s), 1000, true), None);
        assert_eq!(parse_codex("garbage\n\n", 1000, false), None);
    }

    #[test]
    fn rollout_search_and_percent_from_files() {
        let dir = std::env::temp_dir().join(format!("contr-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = dir.join("sessions/2026/11/07");
        let new = dir.join("sessions/2026/11/09");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let line = token_count(Some("codex"), r#"{"used_percent":12.0,"resets_at":99999999999}"#, "null");
        std::fs::write(old.join("rollout-a.jsonl"), token_count(Some("codex"), r#"{"used_percent":77.0}"#, "null")).unwrap();
        std::fs::write(new.join("rollout-b.jsonl"), format!("{line}\n")).unwrap();
        assert_eq!(read_codex(&dir), Some(12));
        assert_eq!(read_codex(&dir.join("missing")), None);
        std::fs::create_dir_all(dir.join("usage")).unwrap();
        std::fs::write(dir.join("usage/claude.json"), r#"{"schema":1,"windows":[{"used_percent":38.0,"resets_at":null}]}"#).unwrap();
        assert_eq!(read_claude(&dir), Some(38));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_label_and_parse() {
        let c = parse_context(r#"{"schema":1,"updated_at":1,"tokens":90000,"size":200000,"model":"opus","effort":"high"}"#).unwrap();
        assert_eq!((c.percent(), c.label().as_str(), c.model.as_str()), (45, "90k of 200k  45%", "opus"));
        let big = Context { tokens: 250_000, size: 1_000_000, model: String::new(), effort: String::new() };
        assert_eq!(big.label(), "250k of 1M  25%");
        assert_eq!(Context { tokens: 5, size: 3, ..big.clone() }.percent(), 100);
        assert_eq!(Context { tokens: 500, size: 1500, ..big }.label(), "500 of 1k  33%");
        assert_eq!(parse_context(r#"{"schema":1,"tokens":1,"size":0}"#), None);
        assert_eq!(parse_context(r#"{"schema":2,"tokens":1,"size":9}"#), None);
        assert_eq!(parse_context("x"), None);
    }
}
