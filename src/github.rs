//! The `gh` commands the app runs (K5), in an agent's worktree. `CONTR_GH` names the program (tests
//! point it at a script); a headless run without it never runs a real `gh`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use rune::Waker;

use crate::hc::model::{Agent, AgentState};

/// The program to run: `CONTR_GH`, else `gh` (not in a headless run).
fn program() -> Option<String> {
    match std::env::var("CONTR_GH") {
        Ok(p) if !p.is_empty() => Some(p),
        _ if rune::is_headless() => None,
        _ => Some("gh".to_owned()),
    }
}

/// `gh <args>` in `dir`: its stdout when it exits 0, else the first line of its stderr.
fn gh(dir: &Path, args: &[&str]) -> Result<String, String> {
    let program = program().ok_or("gh is not available")?;
    let out = Command::new(&program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().next().map(str::trim).filter(|l| !l.is_empty()).unwrap_or("gh failed").to_owned())
}

/// Whether the branch in `dir` has a pull request.
pub fn has_pr(dir: &Path) -> bool {
    gh(dir, &["pr", "view", "--json", "number"]).is_ok()
}

/// Open the branch's pull request in the browser.
pub fn open_pr(dir: &Path) -> Result<(), String> {
    gh(dir, &["pr", "view", "--web"]).map(drop)
}

/// The commit at the head of pull request `pr` (`gh pr view <pr> --json headRefOid`).
pub fn pr_head(dir: &Path, pr: i64) -> Result<String, String> {
    let out = gh(dir, &["pr", "view", &pr.to_string(), "--json", "headRefOid"])?;
    let head: serde_json::Value = serde_json::from_str(&out).map_err(|e| format!("gh pr view: {e}"))?;
    match head["headRefOid"].as_str() {
        Some(sha) if !sha.is_empty() => Ok(sha.to_owned()),
        _ => Err(format!("pull request {pr} has no head commit")),
    }
}

/// The title of issue `n` (`gh issue view <n> --json title`).
pub fn issue_title(dir: &Path, n: &str) -> Result<String, String> {
    parse_title(&gh(dir, &["issue", "view", n, "--json", "title"])?).ok_or_else(|| format!("issue {n} has no title"))
}

/// The title in `gh issue view --json title` output.
fn parse_title(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned)
}

/// Whether the branch in `dir` has an open pull request with unfinished checks. `Ok(false)` without a PR.
pub fn checks_pending(dir: &Path) -> Result<bool, String> {
    match gh(dir, &["pr", "view", "--json", "number,state,statusCheckRollup"]) {
        Ok(out) => Ok(parse_pending(&out)),
        Err(e) if e.contains("no pull requests found") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Whether `gh pr view --json number,state,statusCheckRollup` output is an open PR with a check
/// that has not finished (a run not `COMPLETED`, a status `PENDING` or `EXPECTED`), as Go does (D145).
fn parse_pending(json: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return false };
    if v["state"].as_str() != Some("OPEN") {
        return false;
    }
    v["statusCheckRollup"].as_array().is_some_and(|checks| {
        checks.iter().any(|c| match (c["status"].as_str(), c["state"].as_str()) {
            (Some(status), _) => status != "COMPLETED",
            (None, Some(state)) => matches!(state, "PENDING" | "EXPECTED"),
            _ => false,
        })
    })
}

/// A found title holds this long; a failed read is asked again after `RETRY` (D174).
const TITLE_HOLD: Duration = Duration::from_secs(600);
const RETRY: Duration = Duration::from_secs(300);
/// An answer with pending checks holds a minute (they end soon), any other five (D145).
const CI_PENDING_HOLD: Duration = Duration::from_secs(60);
const CI_HOLD: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Entry {
    issue: Option<String>,
    title: Option<String>,
    title_hold: Option<(Instant, Duration)>,
    ci: bool,
    ci_hold: Option<(Instant, Duration)>,
}

fn fresh(hold: Option<(Instant, Duration)>, now: Instant) -> bool {
    hold.is_some_and(|(at, d)| now.duration_since(at) < d)
}

/// One read for an agent: the issue to look up (`None`: not asked) and whether to read its CI.
struct Job {
    address: String,
    dir: PathBuf,
    issue: Option<String>,
    ci: bool,
}

struct Answer {
    address: String,
    title: Option<Result<String, String>>,
    ci: Option<Result<bool, String>>,
}

/// Issue titles and CI state of the agents, read by `gh` on a thread. Nothing runs without a
/// focused window or without a `gh` to run, and only for an agent with no fresh answer, so an
/// idle screen does nothing.
pub struct Info {
    entries: HashMap<String, Entry>,
    inflight: bool,
    tx: Sender<Vec<Answer>>,
    rx: Receiver<Vec<Answer>>,
}

impl Info {
    pub fn new() -> Info {
        let (tx, rx) = channel();
        Info { entries: HashMap::new(), inflight: false, tx, rx }
    }

    /// The issue title of an agent, once read.
    pub fn title(&self, address: &str) -> Option<&str> {
        self.entries.get(address)?.title.as_deref()
    }

    /// Whether to show `CI` on a row: the agent is live and not working and its PR has unfinished checks.
    pub fn ci(&self, a: &Agent) -> bool {
        !matches!(a.state, AgentState::Working | AgentState::Ended) && self.entries.get(&a.address).is_some_and(|e| e.ci)
    }

    /// Take finished reads and start one for the agents with no fresh answer. Returns whether
    /// something shown changed.
    pub fn poll(&mut self, agents: &[Agent], focused: bool, waker: Option<&Waker>) -> bool {
        let mut changed = false;
        while let Ok(batch) = self.rx.try_recv() {
            self.inflight = false;
            let now = Instant::now();
            for a in batch {
                let Some(e) = self.entries.get_mut(&a.address) else { continue };
                if let Some(t) = a.title {
                    e.title_hold = Some((now, if t.is_ok() { TITLE_HOLD } else { RETRY }));
                    // A failed read keeps what was known.
                    if let Ok(t) = t {
                        changed |= e.title.replace(t.clone()).as_deref() != Some(t.as_str());
                    }
                }
                if let Some(c) = a.ci {
                    let pending = *c.as_ref().unwrap_or(&e.ci);
                    e.ci_hold = Some((now, if c.is_err() { RETRY } else if pending { CI_PENDING_HOLD } else { CI_HOLD }));
                    if let Ok(c) = c {
                        changed |= std::mem::replace(&mut e.ci, c) != c;
                    }
                }
            }
        }
        if !focused || self.inflight || program().is_none() {
            return changed;
        }
        let now = Instant::now();
        self.entries.retain(|k, _| agents.iter().any(|a| &a.address == k));
        let mut jobs = Vec::new();
        for a in agents.iter().filter(|a| a.state != AgentState::Ended) {
            let Some(dir) = a.folder.clone().filter(|f| f.is_dir()) else { continue };
            let e = self.entries.entry(a.address.clone()).or_default();
            if e.issue != a.issue {
                *e = Entry { issue: a.issue.clone(), ..Entry::default() };
            }
            let issue = a.issue.clone().filter(|_| !fresh(e.title_hold, now));
            let ci = a.worktree && !a.is_orchestrator() && !fresh(e.ci_hold, now);
            if issue.is_some() || ci {
                jobs.push(Job { address: a.address.clone(), dir, issue, ci });
            }
        }
        if !jobs.is_empty() {
            self.inflight = true;
            let (out, waker) = (self.tx.clone(), waker.cloned());
            std::thread::spawn(move || {
                let batch = jobs
                    .into_iter()
                    .map(|j| Answer {
                        title: j.issue.as_deref().map(|n| issue_title(&j.dir, n)),
                        ci: j.ci.then(|| checks_pending(&j.dir)),
                        address: j.address,
                    })
                    .collect();
                let _ = out.send(batch);
                if let Some(w) = &waker {
                    w();
                }
            });
        }
        changed
    }
}

/// `owner/repo` of a GitHub remote URL (`<user>@github.com:o/r.git`, `https://github.com/o/r`).
pub fn parse_remote(url: &str) -> Option<String> {
    let path = url.trim().split_once("github.com").map(|(_, p)| p)?;
    let path = path.strip_prefix([':', '/'])?.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && !s.contains('/');
    (ok(owner) && ok(repo)).then(|| path.to_owned())
}

/// `owner/repo` of the `origin` remote of the repo at `dir`, read from its config file (no
/// `git` binary; a worktree's `.git` file leads to the main repo's config). `None` without a
/// GitHub `origin`.
pub fn origin_repo(dir: &Path) -> Option<String> {
    let dot = dir.join(".git");
    let mut base = dot.clone();
    if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let target = Path::new(text.trim().strip_prefix("gitdir:")?.trim()).to_path_buf();
        base = if target.is_absolute() { target } else { dir.join(target) };
        if let Ok(common) = std::fs::read_to_string(base.join("commondir")) {
            base = base.join(common.trim());
        }
    }
    let config = std::fs::read_to_string(base.join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
        } else if in_origin && let Some(url) = line.strip_prefix("url").and_then(|r| r.trim_start().strip_prefix('=')) {
            return parse_remote(url);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_titles_come_from_the_json() {
        for (json, want) in [
            (r#"{"title":"Fix the thing"}"#, Some("Fix the thing")),
            (r#"{"title":"  padded "}"#, Some("padded")),
            (r#"{"title":""}"#, None),
            (r#"{}"#, None),
            ("not json", None),
        ] {
            assert_eq!(parse_title(json).as_deref(), want, "{json}");
        }
    }

    #[test]
    fn checks_are_pending_only_on_an_open_pr_with_an_unfinished_check() {
        let run = |status: &str| format!(r#"{{"status":"{status}","conclusion":""}}"#);
        let ctx = |state: &str| format!(r#"{{"state":"{state}"}}"#);
        let pr = |state: &str, checks: &[String]| format!(r#"{{"number":1,"state":"{state}","statusCheckRollup":[{}]}}"#, checks.join(","));
        for (json, want) in [
            (pr("OPEN", &[run("COMPLETED"), run("COMPLETED")]), false),
            (pr("OPEN", &[run("COMPLETED"), run("IN_PROGRESS")]), true),
            (pr("OPEN", &[run("QUEUED")]), true),
            (pr("OPEN", &[ctx("PENDING")]), true),
            (pr("OPEN", &[ctx("SUCCESS"), ctx("FAILURE")]), false),
            (pr("OPEN", &[]), false),
            (pr("MERGED", &[run("IN_PROGRESS")]), false),
            ("garbage".to_owned(), false),
        ] {
            assert_eq!(parse_pending(&json), want, "{json}");
        }
    }

    #[test]
    fn remotes_parse_to_owner_and_repo() {
        for (url, want) in [
            ("git@github.com:contursofran/contr.git", Some("contursofran/contr")),
            ("https://github.com/contursofran/contr", Some("contursofran/contr")),
            ("https://github.com/o/r.git/", Some("o/r")),
            ("https://gitlab.com/o/r.git", None),
            ("https://github.com/o", None),
        ] {
            assert_eq!(parse_remote(url).as_deref(), want, "{url}");
        }
    }
}
