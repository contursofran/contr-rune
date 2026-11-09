//! Coordinator compaction (PARITY-PLAN D6; Go: cmd/contr/hook.go). Before Claude Code compacts, a
//! coordinator is asked once to write `COORDINATOR-STATE.md`; after the compaction `hook
//! session-start` prints what it needs to carry on: the playbook, the state file, its live
//! workers and its open owner items.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agents::Worker;
use crate::paths;

/// The context size, in tokens, from which a coordinator is asked to save its state: below the
/// default compaction window of 250000 (`CLAUDE_CODE_AUTO_COMPACT_WINDOW`).
const DEFAULT_SAVE_AT: u64 = 220_000;

/// The most characters `session-start` prints. Claude Code caps a hook's context at 10000.
pub const RESTORE_MAX: usize = 9500;

/// The kit's coordinator command; a session that ran it is a coordinator.
pub const ORCHESTRATE_COMMAND: &str = "contr:orchestrate";

/// The coordinator's state file, in its folder.
const STATE_FILE: &str = "COORDINATOR-STATE.md";

/// What the coordinator hooks read from the environment.
#[derive(Default)]
pub struct Env {
    /// `CONTR_STATE_DIR` or the default; the markers live under `hooks/`.
    pub state_dir: Option<PathBuf>,
    /// `owner.json`.
    pub owner_file: Option<PathBuf>,
    /// The kit folder: `CLAUDE_PLUGIN_ROOT`, else `CONTR_KIT`.
    pub kit: Option<PathBuf>,
    /// `agents.json`, for the flow.
    pub config: Option<PathBuf>,
    /// `CONTR_SAVE_AT`, as written.
    pub save_at: String,
}

impl Env {
    pub fn real() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Env {
            state_dir: paths::state_dir(),
            owner_file: paths::owner_file(),
            kit: var("CLAUDE_PLUGIN_ROOT").or_else(|| var("CONTR_KIT")).map(PathBuf::from),
            config: paths::agents_config(),
            save_at: var("CONTR_SAVE_AT").map(|v| v.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    /// `CONTR_SAVE_AT`, else `DEFAULT_SAVE_AT`.
    pub fn save_at(&self) -> u64 {
        self.save_at.trim().parse().ok().filter(|&n| n > 0).unwrap_or(DEFAULT_SAVE_AT)
    }

    /// The marker of the session that was asked: `<state>/hooks/asked-<session_id>`.
    fn marker(&self, session: &str) -> Option<PathBuf> {
        Some(self.state_dir.as_ref()?.join("hooks").join(format!("asked-{}", paths::sanitize_id(session))))
    }

    /// Whether the session was asked. A session with no id is never marked.
    pub fn asked(&self, session: &str) -> bool {
        !session.is_empty() && self.marker(session).is_some_and(|m| m.exists())
    }

    pub fn mark_asked(&self, session: &str) -> Result<(), String> {
        if session.is_empty() {
            return Ok(());
        }
        let marker = self.marker(session).ok_or("no state folder to keep the asked marker in")?;
        let dir = marker.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&marker, b"")).map_err(|e| format!("marker {}: {e}", marker.display()))
    }

    /// Forgets the marker, so the next compaction is asked for again. Best effort.
    pub fn clear_asked(&self, session: &str) {
        if let Some(m) = self.marker(session).filter(|_| !session.is_empty()) {
            let _ = std::fs::remove_file(m);
        }
    }
}

/// Whether a coordinator at `tokens` of context is asked to save its state now.
pub fn should_ask(tokens: u64, save_at: u64, asked: bool) -> bool {
    tokens >= save_at && !asked
}

/// The reason that keeps the turn going once.
pub fn ask_reason(tokens: u64) -> String {
    format!(
        "contr: the context is at {}k tokens, and Claude Code compacts it soon. Update {STATE_FILE} now (Ready, Running with worker, issue, worktree and PR, Decisions, Next), so you can carry on from it after the compaction. Then end your turn.",
        tokens / 1000
    )
}

/// Everything `restore_text` prints, already read.
pub struct Restore<'a> {
    pub cwd: &'a str,
    /// This coordinator's terminal.
    pub me: &'a str,
    pub workers: Result<Vec<Worker>, String>,
    /// The open questions and to-dos of all projects, as `owner.json` holds them.
    pub owner: Result<(Vec<Value>, Vec<Value>), String>,
    pub flow: String,
    pub playbook: String,
}

/// What a coordinator gets after a compaction, cut to `RESTORE_MAX` characters. The lists come
/// before the playbook, so a cut takes only playbook text.
pub fn restore_text(r: &Restore) -> String {
    let state = Path::new(r.cwd).join(STATE_FILE);
    let mut text = format!(
        "contr: the conversation was compacted. You are the coordinator, and the playbook follows again.\nFirst read {}: you wrote it before the compaction.\n\n",
        state.display()
    );
    text += &workers_text(&r.workers, r.me);
    text += "\n";
    text += &owner_text(&r.owner, r.me);
    text += "\n";
    text += &r.flow;
    text += "\nThe playbook:\n\n";
    text += &r.playbook;
    cut(&text, RESTORE_MAX)
}

fn workers_text(workers: &Result<Vec<Worker>, String>, me: &str) -> String {
    let workers = match workers {
        Ok(w) => w,
        Err(e) => return format!("Your live workers: unknown ({e}); use the list tool.\n"),
    };
    let mut text = String::from("Your live workers:\n");
    let mine: Vec<&Worker> = workers.iter().filter(|w| !me.is_empty() && w.parent == me).collect();
    for w in &mine {
        let status = if w.status.trim().is_empty() { "no status" } else { &w.status };
        text += &format!("- {} ({}) {}, {status}", w.name, w.address, w.role);
        if !w.issue.is_empty() {
            text += &format!(", issue {}", w.issue);
        }
        if !w.worktree.is_empty() {
            text += &format!(", {}", w.worktree);
        }
        text += "\n";
    }
    if mine.is_empty() {
        text += "- none\n";
    }
    text
}

fn owner_text(owner: &Result<(Vec<Value>, Vec<Value>), String>, me: &str) -> String {
    let (questions, todos) = match owner {
        Ok(items) => items,
        Err(e) => return format!("Open owner items: unknown ({e}); use the owner_items tool.\n"),
    };
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let mine = |v: &&Value| text(v, "coordinator") == me;
    let mut lines = Vec::new();
    for q in questions.iter().filter(mine) {
        let title = if text(q, "title").trim().is_empty() { text(q, "question") } else { text(q, "title") };
        lines.push(format!("- {} {}\n", text(q, "id"), one_line(&title)));
    }
    for t in todos.iter().filter(mine) {
        lines.push(format!("- {} [{}] {}\n", text(t, "id"), text(t, "kind"), one_line(&text(t, "text"))));
    }
    if lines.is_empty() {
        lines.push("- none\n".to_owned());
    }
    format!("Your open owner items:\n{}", lines.concat())
}

/// The body of the kit's orchestrate command, without its front matter. The goal is in the state
/// file.
pub fn playbook(kit: Option<&Path>) -> String {
    let read = match kit {
        None => Err("no kit folder".to_owned()),
        Some(kit) => std::fs::read_to_string(kit.join("commands/orchestrate.md")).map_err(|e| e.to_string()),
    };
    match read {
        Ok(body) => strip_front_matter(&body).replace("$ARGUMENTS", "(see the state file)").trim().to_owned(),
        Err(e) => format!("The playbook could not be read ({e}). Run /contr:orchestrate with your goal to read it again."),
    }
}

fn strip_front_matter(body: &str) -> &str {
    body.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")).map_or(body, |(_, after)| after)
}

fn one_line(s: &str) -> String {
    cut(&s.split_whitespace().collect::<Vec<_>>().join(" "), 160)
}

/// At most `n` characters of `s`, with `…` as the last one when it cuts.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    s.chars().take(n - 1).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("contr-rune-restore-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_at_table() {
        for (set, want) in [("", 220_000), ("150000", 150_000), (" 90000 ", 90_000), ("0", 220_000), ("-5", 220_000), ("lots", 220_000)] {
            assert_eq!(Env { save_at: set.into(), ..Env::default() }.save_at(), want, "{set:?}");
        }
    }

    #[test]
    fn should_ask_table() {
        for (tokens, asked, want) in [(219_999, false, false), (220_000, false, true), (300_000, false, true), (300_000, true, false), (0, false, false)] {
            assert_eq!(should_ask(tokens, 220_000, asked), want, "{tokens} {asked}");
        }
    }

    #[test]
    fn the_marker_is_per_session_and_cleared_by_name() {
        let dir = scratch("marker");
        let env = Env { state_dir: Some(dir.clone()), ..Env::default() };
        assert!(!env.asked("s1"));
        env.mark_asked("s1").unwrap();
        assert!(env.asked("s1") && !env.asked("s2"));
        assert!(dir.join("hooks/asked-s1").exists());
        env.mark_asked("../x").unwrap();
        assert!(dir.join("hooks/asked-___x").exists());
        env.clear_asked("s1");
        assert!(!env.asked("s1") && env.asked("../x"));
        // No session id: nothing is kept, so it is never "asked".
        env.mark_asked("").unwrap();
        assert!(!env.asked(""));
        assert!(Env::default().mark_asked("s").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn worker(name: &str, parent: &str, status: &str, issue: &str) -> Worker {
        let terminal = json!({"address": "t9", "name": name, "state": "live", "turn": 1, "parent": parent, "labels": {"role": "builder", "issue": issue, "worktree": "/wt/a"}});
        let mut w = Worker::from(&serde_json::from_value(terminal).unwrap());
        w.status = status.into();
        w
    }

    fn sample<'a>() -> Restore<'a> {
        Restore {
            cwd: "/proj",
            me: "t3",
            workers: Ok(vec![worker("b1", "t3", "working", "#7"), worker("other", "t4", "idle", ""), worker("b2", "t3", "", "")]),
            owner: Ok((
                vec![json!({"id": "D1", "coordinator": "t3", "title": "", "question": "ship\n it?"}), json!({"id": "D2", "coordinator": "t4", "title": "x"})],
                vec![json!({"id": "T1", "coordinator": "t3", "kind": "do", "text": "run it"})],
            )),
            flow: "Flow: plan first\n".into(),
            playbook: "PLAYBOOK".into(),
        }
    }

    #[test]
    fn restore_text_lists_only_my_workers_and_items_before_the_playbook() {
        let want = "contr: the conversation was compacted. You are the coordinator, and the playbook follows again.\n\
First read /proj/COORDINATOR-STATE.md: you wrote it before the compaction.\n\n\
Your live workers:\n- b1 (t9) builder, working, issue #7, /wt/a\n- b2 (t9) builder, no status, /wt/a\n\n\
Your open owner items:\n- D1 ship it?\n- T1 [do] run it\n\n\
Flow: plan first\n\nThe playbook:\n\nPLAYBOOK";
        assert_eq!(restore_text(&sample()), want);
    }

    #[test]
    fn restore_text_says_none_and_unknown() {
        let mut r = sample();
        r.workers = Ok(vec![]);
        r.owner = Err("no file".into());
        let text = restore_text(&r);
        assert!(text.contains("Your live workers:\n- none\n"), "{text}");
        assert!(text.contains("Open owner items: unknown (no file); use the owner_items tool."));
        r.workers = Err("hc is off".into());
        r.owner = Ok((vec![], vec![]));
        let text = restore_text(&r);
        assert!(text.contains("Your live workers: unknown (hc is off); use the list tool."));
        assert!(text.contains("Your open owner items:\n- none\n"));
    }

    #[test]
    fn restore_text_is_cut_to_the_cap_inside_the_playbook() {
        let mut r = sample();
        r.playbook = "p".repeat(20_000);
        let text = restore_text(&r);
        assert_eq!(text.chars().count(), RESTORE_MAX);
        assert!(text.ends_with('…'));
        assert!(text.contains("Your open owner items:\n- D1"));
        // Multi-byte text is cut by characters, never in the middle of one.
        r.playbook = "é".repeat(20_000);
        assert_eq!(restore_text(&r).chars().count(), RESTORE_MAX);
    }

    #[test]
    fn playbook_drops_the_front_matter_and_fills_the_goal() {
        let dir = scratch("playbook");
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::write(dir.join("commands/orchestrate.md"), "---\ndescription: x\n---\n\nGoal: $ARGUMENTS\nGo.\n").unwrap();
        assert_eq!(playbook(Some(&dir)), "Goal: (see the state file)\nGo.");
        std::fs::write(dir.join("commands/orchestrate.md"), "no front matter\n").unwrap();
        assert_eq!(playbook(Some(&dir)), "no front matter");
        assert!(playbook(Some(&dir.join("missing"))).starts_with("The playbook could not be read ("));
        assert!(playbook(None).contains("(no kit folder)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_reason_names_the_thousands_and_the_state_file() {
        let reason = ask_reason(231_456);
        assert!(reason.starts_with("contr: the context is at 231k tokens") && reason.contains("Update COORDINATOR-STATE.md now"));
    }
}
