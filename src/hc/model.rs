//! Agents derived from hc terminals (Contract C6), and the event apply the watcher uses to keep the
//! terminal list current (C4). Pure: no clock, no files; the caller passes `now` and the folder
//! checks.
//!
//! A terminal is an agent row when it carries `contr=agent` and a role other than `terminal`, or
//! when it is foreign (no `contr=agent`) and Claude runs in it (then it counts as an orchestrator,
//! C5). Plain terminals and this app's own tabs (`role=terminal`) are not agents.
//!
//! The project of an agent: the `project` label when it names an open project, else the open
//! project whose folder holds the agent's `worktree` label (else `cwd`), the deepest one winning,
//! else its parent's project. An agent with none keeps its `folder` so the app can open it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::restore::Known;
use super::wire::{Event, Terminal};

/// What an agent is doing. Needs-you wins over working; an orchestrator waits while a live child
/// works; `Ended` is a finished terminal. (H6 makes this live; H5 maps the record as it stands.)
#[derive(Clone, Debug, PartialEq)]
pub enum AgentState {
    NeedsYou { kind: Option<String>, msg: String },
    Working,
    Waiting,
    Idle,
    Ended,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub address: String,
    /// The terminal's name, else its address.
    pub name: String,
    pub parent: Option<String>,
    /// `orchestrator`, or a worker profile (`builder`, ...).
    pub role: String,
    /// Index into the known projects.
    pub project: Option<usize>,
    /// The start folder (`worktree` label, else `cwd`).
    pub folder: Option<PathBuf>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title: String,
    /// Time since the last activity; `None` when no stamp is known (0 or missing).
    pub age_ms: Option<u64>,
    pub state: AgentState,
    pub created_ms: u64,
    /// No `contr=agent` label: found by the Claude process in it.
    pub foreign: bool,
}

impl Agent {
    pub fn is_orchestrator(&self) -> bool {
        self.role == "orchestrator"
    }

    /// The row's title: the terminal's title, else its name.
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() { &self.name } else { &self.title }
    }
}

fn label<'a>(t: &'a Terminal, key: &str) -> Option<&'a str> {
    t.labels.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

fn is_version(s: &str) -> bool {
    s.contains('.') && s.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Claude runs in `t`: foreground `claude` or a version number (`2.1.30`), argv[0] `claude`, or it
/// has reported a status.
pub fn runs_claude(t: &Terminal) -> bool {
    let fg = t.foreground.as_ref().is_some_and(|f| f.name == "claude" || is_version(&f.name));
    let argv0 = t.argv.first().is_some_and(|a| Path::new(a).file_name().is_some_and(|n| n == "claude"));
    fg || argv0 || t.status.is_some()
}

/// The value of `--flag X` or `--flag=X` in `argv`.
fn flag(argv: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned().filter(|v| !v.is_empty());
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_owned()).filter(|v| !v.is_empty());
        }
    }
    None
}

fn number(address: &str) -> u64 {
    address.trim_start_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(u64::MAX)
}

fn folder(t: &Terminal) -> Option<PathBuf> {
    let w = label(t, "worktree");
    w.or(Some(t.cwd.as_str()).filter(|c| !c.is_empty())).map(PathBuf::from)
}

/// The agents among `terminals`, oldest first. `stamps` is the time of the last event per
/// terminal (else `status_ms` stands in); `within(project, folder)` says whether `folder` is in
/// `project`'s folder.
pub fn derive(terminals: &[Terminal], stamps: &HashMap<String, u64>, known: &[Known], within: &dyn Fn(&Path, &Path) -> bool, now: u64) -> Vec<Agent> {
    let mut agents: Vec<Agent> = Vec::new();
    for t in terminals {
        let ours = label(t, "contr") == Some("agent");
        let role = if ours {
            match label(t, "role") {
                Some("terminal") => continue,
                Some(r) => r.to_owned(),
                None => "worker".to_owned(),
            }
        } else if runs_claude(t) {
            "orchestrator".to_owned()
        } else {
            continue;
        };
        let folder = folder(t);
        let by_label = label(t, "project").and_then(|n| known.iter().position(|k| k.name == n));
        let by_path = || {
            let f = folder.as_deref()?;
            known.iter().enumerate().filter(|(_, k)| within(k.path, f)).max_by_key(|(_, k)| k.path.components().count()).map(|(i, _)| i)
        };
        // A 0 stamp means unknown: fall back to the status stamp, then `created_ms`, else no age.
        let stamp = [stamps.get(&t.address).copied().unwrap_or(0), t.status_ms, t.created_ms].into_iter().find(|s| *s > 0);
        let state = if t.state == "finished" {
            AgentState::Ended
        } else {
            match t.status.as_ref().map(|s| (s.state.as_str(), s)) {
                Some(("blocked", s)) => AgentState::NeedsYou { kind: s.kind.clone(), msg: s.msg.clone() },
                Some(("working", _)) => AgentState::Working,
                _ => AgentState::Idle,
            }
        };
        agents.push(Agent {
            address: t.address.clone(),
            name: if t.name.is_empty() { t.address.clone() } else { t.name.clone() },
            parent: t.parent.clone().filter(|p| !p.is_empty()),
            role,
            project: by_label.or_else(by_path),
            folder,
            model: flag(&t.argv, "--model").or_else(|| label(t, "contr.model").map(str::to_owned)),
            effort: flag(&t.argv, "--effort").or_else(|| label(t, "contr.effort").map(str::to_owned)),
            title: t.title.clone(),
            age_ms: stamp.map(|s| now.saturating_sub(s)),
            state,
            created_ms: t.created_ms,
            foreign: !ours,
        });
    }
    agents.sort_by_key(|a| (a.created_ms, number(&a.address)));
    // A child without a project of its own takes its parent's (up the chain).
    for i in 0..agents.len() {
        let mut at = agents[i].parent.clone();
        let mut hops = 0;
        while agents[i].project.is_none() && hops < 16 {
            let Some(p) = at.take() else { break };
            let Some((project, up)) = agents.iter().find(|a| a.address == p).map(|a| (a.project, a.parent.clone())) else { break };
            agents[i].project = project;
            at = up;
            hops += 1;
        }
    }
    // An orchestrator that has a live child at work is waiting for it.
    let busy: HashSet<String> = agents.iter().filter(|a| a.state == AgentState::Working).filter_map(|a| a.parent.clone()).collect();
    for a in &mut agents {
        if a.is_orchestrator() && a.state == AgentState::Idle && busy.contains(&a.address) {
            a.state = AgentState::Waiting;
        }
    }
    agents
}

/// A row of the sidebar: the agent, its indent (1 under the project, 2 for a worker) and how many
/// live children it has.
#[derive(Debug, PartialEq)]
pub struct Line<'a> {
    pub agent: &'a Agent,
    pub depth: u8,
    pub children: usize,
}

/// The rows under project `project`: its top agents in order, each followed by its children.
/// Ended agents are left out. An agent whose parent is not shown there stands at the top.
pub fn lines(agents: &[Agent], project: usize) -> Vec<Line<'_>> {
    let shown: Vec<&Agent> = agents.iter().filter(|a| a.project == Some(project) && a.state != AgentState::Ended).collect();
    let has = |addr: &str| shown.iter().any(|a| a.address == addr);
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    fn walk<'a>(a: &'a Agent, depth: u8, shown: &[&'a Agent], seen: &mut HashSet<&'a str>, out: &mut Vec<Line<'a>>) {
        if !seen.insert(&a.address) {
            return;
        }
        let kids: Vec<&Agent> = shown.iter().copied().filter(|k| k.parent.as_deref() == Some(a.address.as_str())).collect();
        out.push(Line { agent: a, depth, children: kids.len() });
        for k in kids {
            walk(k, depth + 1, shown, seen, out);
        }
    }
    for a in shown.iter().copied().filter(|a| a.parent.as_deref().is_none_or(|p| !has(p))) {
        walk(a, 1, &shown, &mut seen, &mut out);
    }
    out
}

/// The row's second line: a worker shows `name  model` (`b-241  sonnet`); an orchestrator shows
/// its model, else `N workers`, else `orchestrator`, and `  idle` after it while it is idle.
pub fn meta(line: &Line) -> String {
    let a = line.agent;
    if a.is_orchestrator() {
        let what = match (&a.model, line.children) {
            (Some(m), _) => m.clone(),
            (None, 0) => "orchestrator".to_owned(),
            (None, 1) => "1 worker".to_owned(),
            (None, n) => format!("{n} workers"),
        };
        return if a.state == AgentState::Idle { format!("{what}  idle") } else { what };
    }
    match &a.model {
        Some(m) => format!("{}  {m}", a.name),
        None => a.name.clone(),
    }
}

fn labels_of(v: &Value) -> Option<std::collections::BTreeMap<String, String>> {
    let o = v.as_object()?;
    Some(o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
}

/// Whether an event of this kind is the terminal doing something, so its age starts again.
/// Size, foreground and label changes are not (the core reads the foreground about twice a second).
pub fn is_activity(kind: &str) -> bool {
    matches!(kind, "created" | "restarted" | "finished" | "lost" | "status" | "turn" | "title")
}

/// Apply one event to the terminal list (C4). Returns whether the list changed. `created` and
/// `restarted` carry the whole record; `finished` and `lost` drop the foreground; `removed`
/// drops the record; `status`, `turn`, `title`, `labels`, `size` and `foreground` set that field.
/// `keep` and `doc` change nothing we show.
pub fn apply(list: &mut Vec<Terminal>, ev: &Event) -> bool {
    let Some(addr) = ev.terminal.as_deref() else { return false };
    let at = list.iter().position(|t| t.address == addr);
    match ev.kind.as_str() {
        "created" | "restarted" => {
            let Ok(t) = serde_json::from_value::<Terminal>(ev.data.clone()) else { return false };
            match at {
                Some(i) => list[i] = t,
                None => list.push(t),
            }
            true
        }
        "removed" => at.map(|i| list.remove(i)).is_some(),
        "finished" | "lost" => {
            let Some(i) = at else { return false };
            let t = &mut list[i];
            t.foreground = None;
            if ev.kind == "finished" {
                t.state = "finished".into();
                t.status = None;
                t.reason = ev.data["reason"].as_str().map(str::to_owned);
                t.exit_code = ev.data["exit_code"].as_i64();
            }
            true
        }
        "title" => {
            let (Some(i), Some(title)) = (at, ev.data["title"].as_str()) else { return false };
            list[i].title = title.to_owned();
            true
        }
        "labels" => {
            let (Some(i), Some(l)) = (at, labels_of(&ev.data["labels"])) else { return false };
            list[i].labels = l;
            true
        }
        "status" => {
            // `status` is the root status, null when cleared; a missing key is not an update.
            let (Some(i), Some(v)) = (at, ev.data.get("status")) else { return false };
            let status = if v.is_null() {
                None
            } else {
                match serde_json::from_value(v.clone()) {
                    Ok(s) => Some(s),
                    Err(_) => return false,
                }
            };
            list[i].status = status;
            true
        }
        "turn" => {
            let (Some(i), Some(n)) = (at, ev.data["turn"].as_u64()) else { return false };
            list[i].turn = n;
            true
        }
        "size" => {
            let (Some(i), Some(c), Some(r)) = (at, ev.data["cols"].as_u64(), ev.data["rows"].as_u64()) else { return false };
            list[i].cols = c as u16;
            list[i].rows = r as u16;
            true
        }
        "foreground" => {
            let Some(i) = at else { return false };
            let fg = if ev.data.is_null() {
                None
            } else {
                match serde_json::from_value(ev.data.clone()) {
                    Ok(f) => Some(f),
                    Err(_) => return false,
                }
            };
            list[i].foreground = fg;
            true
        }
        _ => false,
    }
}

/// The terminals the core has, kept current from its notifications (C4): the snapshot, then each
/// event in order. `seq` rises by one per event; an event at or below the last one seen (a repeat,
/// or one the snapshot already holds) is dropped. A `missed` snapshot replaces the list and sets
/// the floor. `seq` restarts with each core, so a new connection starts a new `Live`.
pub struct Live {
    pub terminals: Vec<Terminal>,
    /// The time (ms) of each terminal's last activity.
    pub stamps: HashMap<String, u64>,
    seen: u64,
}

impl Live {
    pub fn new(terminals: Vec<Terminal>, seq: u64) -> Live {
        Live { terminals, stamps: HashMap::new(), seen: seq }
    }

    /// Take one event at time `now`. Returns whether the terminal list changed.
    pub fn event(&mut self, ev: &Event, now: u64) -> bool {
        if ev.seq <= self.seen {
            return false;
        }
        self.seen = ev.seq;
        let Some(addr) = ev.terminal.as_deref() else { return false };
        let changed = apply(&mut self.terminals, ev);
        if !self.terminals.iter().any(|t| t.address == addr) {
            self.stamps.remove(addr);
        } else if changed && is_activity(&ev.kind) {
            self.stamps.insert(addr.to_owned(), now);
        }
        changed
    }

    /// Take a `missed` snapshot: it replaces the list, and events up to `seq` are history. A
    /// terminal that differs from what we had counts as active now; one that went has no stamp.
    pub fn missed(&mut self, seq: u64, terminals: Vec<Terminal>, now: u64) -> bool {
        self.seen = self.seen.max(seq);
        let changed = self.terminals != terminals;
        for t in &terminals {
            if self.terminals.iter().find(|o| o.address == t.address) != Some(t) {
                self.stamps.insert(t.address.clone(), now);
            }
        }
        self.stamps.retain(|a, _| terminals.iter().any(|t| &t.address == a));
        self.terminals = terminals;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::wire::{Foreground, Status};
    use serde_json::json;

    fn t(address: &str, created: u64, labels: &[(&str, &str)]) -> Terminal {
        let mut t = Terminal { address: address.into(), name: format!("n-{address}"), state: "live".into(), created_ms: created, status_ms: created, ..Terminal::default() };
        for (k, v) in labels {
            t.labels.insert((*k).into(), (*v).into());
        }
        t
    }

    fn agent_t(address: &str, created: u64, role: &str, project: &str) -> Terminal {
        t(address, created, &[("contr", "agent"), ("role", role), ("project", project)])
    }

    const KNOWN: [(&str, &str); 2] = [("a", "/p/a"), ("b", "/p/b")];

    fn run(ts: &[Terminal], now: u64) -> Vec<Agent> {
        let known: Vec<Known> = KNOWN.iter().map(|(n, p)| Known { name: n, path: Path::new(p) }).collect();
        derive(ts, &HashMap::new(), &known, &|proj, f| f.starts_with(proj), now)
    }

    fn status(state: &str, kind: Option<&str>) -> Option<Status> {
        Some(Status { state: state.into(), kind: kind.map(str::to_owned), msg: "ok?".into(), ..Status::default() })
    }

    #[test]
    fn tabs_and_plain_terminals_are_not_agents() {
        let ts = [agent_t("t1", 1, "terminal", "a"), t("t2", 2, &[]), agent_t("t3", 3, "orchestrator", "a")];
        let a = run(&ts, 10);
        assert_eq!(a.iter().map(|a| a.address.as_str()).collect::<Vec<_>>(), vec!["t3"]);
        assert!(!a[0].foreign);
    }

    #[test]
    fn a_foreign_claude_terminal_is_an_orchestrator() {
        let mut by_fg = t("t1", 1, &[]);
        by_fg.foreground = Some(Foreground { name: "claude".into(), ..Foreground::default() });
        let mut by_version = t("t2", 2, &[]);
        by_version.foreground = Some(Foreground { name: "2.1.30".into(), ..Foreground::default() });
        let mut by_argv = t("t3", 3, &[]);
        by_argv.argv = vec!["/usr/local/bin/claude".into()];
        let mut by_status = t("t4", 4, &[]);
        by_status.status = status("idle", None);
        let mut zsh = t("t5", 5, &[]);
        zsh.foreground = Some(Foreground { name: "zsh".into(), ..Foreground::default() });
        let a = run(&[by_fg, by_version, by_argv, by_status, zsh], 10);
        assert_eq!(a.iter().map(|a| a.address.as_str()).collect::<Vec<_>>(), vec!["t1", "t2", "t3", "t4"]);
        assert!(a.iter().all(|a| a.foreign && a.role == "orchestrator"));
    }

    #[test]
    fn model_and_effort_come_from_argv_then_labels() {
        let mut one = agent_t("t1", 1, "builder", "a");
        one.argv = vec!["claude".into(), "--model".into(), "sonnet".into(), "--effort=high".into()];
        one.labels.insert("contr.model".into(), "opus".into());
        let mut two = agent_t("t2", 2, "builder", "a");
        two.labels.insert("contr.model".into(), "opus".into());
        two.labels.insert("contr.effort".into(), "low".into());
        let three = agent_t("t3", 3, "builder", "a");
        let a = run(&[one, two, three], 10);
        assert_eq!((a[0].model.as_deref(), a[0].effort.as_deref()), (Some("sonnet"), Some("high")));
        assert_eq!((a[1].model.as_deref(), a[1].effort.as_deref()), (Some("opus"), Some("low")));
        assert_eq!((a[2].model.as_deref(), a[2].effort.as_deref()), (None, None));
    }

    #[test]
    fn the_name_falls_back_to_the_address_and_the_title_to_the_name() {
        let mut x = agent_t("t9", 1, "builder", "a");
        x.name = String::new();
        let a = run(&[x], 10);
        assert_eq!(a[0].name, "t9");
        assert_eq!(a[0].display_title(), "t9");
        let mut y = agent_t("t8", 1, "builder", "a");
        y.title = "Build it".into();
        assert_eq!(run(&[y], 10)[0].display_title(), "Build it");
    }

    #[test]
    fn the_project_is_the_label_then_the_deepest_folder_then_the_parent() {
        let mut by_path = t("t1", 1, &[("contr", "agent"), ("role", "orchestrator")]);
        by_path.cwd = "/p/b/sub".into();
        let mut by_worktree = t("t2", 2, &[("contr", "agent"), ("role", "builder"), ("project", "zzz"), ("worktree", "/p/a/wt")]);
        by_worktree.cwd = "/elsewhere".into();
        let by_label = agent_t("t3", 3, "builder", "b");
        let mut child = t("t4", 4, &[("contr", "agent"), ("role", "builder"), ("project", "nope")]);
        child.parent = Some("t1".into());
        child.cwd = "/gone".into();
        let mut lost = t("t5", 5, &[("contr", "agent"), ("role", "orchestrator"), ("project", "nope")]);
        lost.cwd = "/gone".into();
        let a = run(&[by_path, by_worktree, by_label, child, lost], 10);
        assert_eq!(a.iter().map(|a| a.project).collect::<Vec<_>>(), vec![Some(1), Some(0), Some(1), Some(1), None]);
        assert_eq!(a[4].folder.as_deref(), Some(Path::new("/gone")), "an unmatched agent keeps its folder");
    }

    #[test]
    fn the_deepest_project_wins() {
        let known = [Known { name: "home", path: Path::new("/h") }, Known { name: "deep", path: Path::new("/h/deep") }];
        let mut x = t("t1", 1, &[("contr", "agent"), ("role", "orchestrator")]);
        x.cwd = "/h/deep/src".into();
        let a = derive(&[x], &HashMap::new(), &known, &|p, f| f.starts_with(p), 0);
        assert_eq!(a[0].project, Some(1));
    }

    #[test]
    fn state_maps_the_status_and_orchestrators_wait_for_busy_children() {
        let mut orch = agent_t("t1", 1, "orchestrator", "a");
        orch.status = status("idle", None);
        let mut w = agent_t("t2", 2, "builder", "a");
        w.parent = Some("t1".into());
        w.status = status("working", None);
        let mut blocked = agent_t("t3", 3, "builder", "a");
        blocked.status = status("blocked", Some("permission"));
        let mut done = agent_t("t4", 4, "builder", "a");
        done.state = "finished".into();
        done.status = status("working", None);
        let silent = agent_t("t5", 5, "builder", "a");
        let a = run(&[orch.clone(), w.clone(), blocked, done, silent], 10);
        assert_eq!(a[0].state, AgentState::Waiting);
        assert_eq!(a[1].state, AgentState::Working);
        assert_eq!(a[2].state, AgentState::NeedsYou { kind: Some("permission".into()), msg: "ok?".into() });
        assert_eq!(a[3].state, AgentState::Ended);
        assert_eq!(a[4].state, AgentState::Idle);
        // Without a busy child the orchestrator is idle; a blocked orchestrator needs you.
        assert_eq!(run(&[orch.clone()], 10)[0].state, AgentState::Idle);
        let mut b = orch;
        b.status = status("blocked", None);
        w.state = "live".into();
        assert!(matches!(run(&[b, w], 10)[0].state, AgentState::NeedsYou { .. }));
    }

    #[test]
    fn age_is_since_the_last_event_else_the_status_stamp() {
        let mut x = agent_t("t1", 1, "builder", "a");
        x.status_ms = 4_000;
        let y = agent_t("t2", 2, "builder", "a");
        let known: Vec<Known> = KNOWN.iter().map(|(n, p)| Known { name: n, path: Path::new(p) }).collect();
        let stamps = HashMap::from([("t2".to_owned(), 9_000)]);
        let a = derive(&[x, y], &stamps, &known, &|p, f| f.starts_with(p), 10_000);
        assert_eq!((a[0].age_ms, a[1].age_ms), (Some(6_000), Some(1_000)));
        let future = derive(&[agent_t("t3", 1, "builder", "a")], &HashMap::from([("t3".into(), 99_000)]), &known, &|p, f| f.starts_with(p), 10_000);
        assert_eq!(future[0].age_ms, Some(0), "a stamp ahead of the clock is age 0");
        // Missing stamps: created_ms when set, else no age.
        let mut z = agent_t("t4", 0, "builder", "a");
        z.status_ms = 0;
        z.created_ms = 0;
        let mut c = agent_t("t5", 0, "builder", "a");
        c.status_ms = 0;
        c.created_ms = 7_000;
        let a = derive(&[z, c], &HashMap::new(), &known, &|p, f| f.starts_with(p), 10_000);
        assert_eq!((a[0].age_ms, a[1].age_ms), (None, Some(3_000)), "a 0 stamp is unknown, not 1970");
    }

    fn tree() -> Vec<Agent> {
        let orch = agent_t("t3", 1, "orchestrator", "a");
        let mut w1 = agent_t("t17", 2, "builder", "a");
        w1.parent = Some("t3".into());
        let mut w2 = agent_t("t18", 3, "builder", "a");
        w2.parent = Some("t3".into());
        let other = agent_t("t30", 0, "orchestrator", "b");
        let mut orphan = agent_t("t31", 4, "builder", "a");
        orphan.parent = Some("t99".into());
        run(&[w2, orphan, orch, w1, other], 10)
    }

    #[test]
    fn lines_nest_workers_under_their_orchestrator_in_creation_order() {
        let a = tree();
        let rows: Vec<(String, u8, usize)> = lines(&a, 0).iter().map(|l| (l.agent.address.clone(), l.depth, l.children)).collect();
        assert_eq!(rows, vec![("t3".into(), 1, 2), ("t17".into(), 2, 0), ("t18".into(), 2, 0), ("t31".into(), 1, 0)]);
        let b: Vec<String> = lines(&a, 1).iter().map(|l| l.agent.address.clone()).collect();
        assert_eq!(b, vec!["t30"]);
        assert!(lines(&a, 5).is_empty());
    }

    #[test]
    fn ended_agents_have_no_row_and_their_children_move_up() {
        let orch = agent_t("t1", 1, "orchestrator", "a");
        let mut w = agent_t("t2", 2, "builder", "a");
        w.parent = Some("t1".into());
        let mut gone = orch.clone();
        gone.state = "finished".into();
        let a = run(&[gone, w], 10);
        let rows: Vec<(String, u8)> = lines(&a, 0).iter().map(|l| (l.agent.address.clone(), l.depth)).collect();
        assert_eq!(rows, vec![("t2".into(), 1)]);
    }

    #[test]
    fn a_parent_cycle_does_not_loop() {
        let mut x = agent_t("t1", 1, "builder", "a");
        x.parent = Some("t2".into());
        let mut y = agent_t("t2", 2, "builder", "a");
        y.parent = Some("t1".into());
        let a = run(&[x, y], 10);
        assert!(lines(&a, 0).len() <= 2);
    }

    #[test]
    fn meta_text() {
        let mut w = agent_t("t17", 1, "builder", "a");
        w.name = "b-241".into();
        w.argv = vec!["claude".into(), "--model".into(), "sonnet".into()];
        let mut bare = agent_t("t18", 2, "builder", "a");
        bare.name = "b-242".into();
        let mut orch = agent_t("t3", 3, "orchestrator", "a");
        orch.labels.insert("contr.model".into(), "opus".into());
        let no_model = agent_t("t4", 4, "orchestrator", "b");
        let mut kid = agent_t("t5", 5, "builder", "b");
        kid.parent = Some("t4".into());
        let mut kid2 = agent_t("t6", 6, "builder", "b");
        kid2.parent = Some("t4".into());
        let a = run(&[w, bare, orch, no_model, kid, kid2], 10);
        let m = |addr: &str, project: usize| meta(lines(&a, project).iter().find(|l| l.agent.address == addr).unwrap());
        assert_eq!(m("t17", 0), "b-241  sonnet");
        assert_eq!(m("t18", 0), "b-242");
        assert_eq!(m("t3", 0), "opus  idle");
        assert_eq!(m("t4", 1), "2 workers  idle");
        let solo = run(&[agent_t("t7", 1, "orchestrator", "a")], 10);
        assert_eq!(meta(&lines(&solo, 0)[0]), "orchestrator  idle");
    }

    fn ev(kind: &str, terminal: Option<&str>, data: Value) -> Event {
        Event { seq: 1, kind: kind.into(), terminal: terminal.map(str::to_owned), data }
    }

    #[test]
    fn created_adds_restarted_replaces_removed_drops() {
        let mut list = vec![];
        let rec = |title: &str| json!({"address": "t1", "state": "live", "title": title, "labels": {"contr": "agent"}});
        assert!(apply(&mut list, &ev("created", Some("t1"), rec("one"))));
        assert!(apply(&mut list, &ev("restarted", Some("t1"), rec("two"))));
        assert_eq!((list.len(), list[0].title.as_str()), (1, "two"));
        assert!(!apply(&mut list, &ev("removed", Some("t9"), json!({}))));
        assert!(apply(&mut list, &ev("removed", Some("t1"), json!({}))));
        assert!(list.is_empty());
        assert!(!apply(&mut list, &ev("created", Some("t1"), json!("not a record"))));
        assert!(!apply(&mut list, &ev("created", None, rec("x"))));
    }

    #[test]
    fn finished_ends_the_record_and_lost_only_drops_the_foreground() {
        let mut one = agent_t("t1", 1, "builder", "a");
        one.foreground = Some(Foreground { name: "claude".into(), ..Foreground::default() });
        one.status = status("working", None);
        let mut list = vec![one];
        assert!(apply(&mut list, &ev("lost", Some("t1"), json!({"reason": "lost"}))));
        assert!(list[0].foreground.is_none() && list[0].state == "live" && list[0].status.is_some());
        assert!(apply(&mut list, &ev("finished", Some("t1"), json!({"reason": "exited", "exit_code": 3}))));
        assert_eq!((list[0].state.as_str(), list[0].reason.as_deref(), list[0].exit_code), ("finished", Some("exited"), Some(3)));
        assert!(list[0].status.is_none());
        assert!(!apply(&mut list, &ev("finished", Some("t7"), json!({}))));
    }

    #[test]
    fn title_and_labels_update_in_place_and_other_kinds_are_ignored() {
        let mut list = vec![agent_t("t1", 1, "builder", "a")];
        assert!(apply(&mut list, &ev("title", Some("t1"), json!({"title": "New"}))));
        assert_eq!(list[0].title, "New");
        assert!(apply(&mut list, &ev("labels", Some("t1"), json!({"labels": {"role": "reviewer"}}))));
        assert_eq!(list[0].labels.get("role").map(String::as_str), Some("reviewer"));
        assert!(!apply(&mut list, &ev("title", Some("t1"), json!({}))));
        assert!(!apply(&mut list, &ev("keep", Some("t1"), json!({"keep": true}))));
        assert!(!apply(&mut list, &ev("doc", None, json!({}))));
    }

    #[test]
    fn status_sets_replaces_and_clears_the_status() {
        let mut list = vec![agent_t("t1", 1, "builder", "a")];
        let blocked = json!({"status": {"state": "blocked", "kind": "permission", "msg": "Run it?"}, "records": {}, "reported": true});
        assert!(apply(&mut list, &ev("status", Some("t1"), blocked)));
        let s = list[0].status.clone().unwrap();
        assert_eq!((s.state.as_str(), s.kind.as_deref(), s.msg.as_str()), ("blocked", Some("permission"), "Run it?"));
        assert!(apply(&mut list, &ev("status", Some("t1"), json!({"status": {"state": "working"}, "records": {}, "reported": true}))));
        assert_eq!(list[0].status.as_ref().map(|s| s.state.as_str()), Some("working"));
        assert!(apply(&mut list, &ev("status", Some("t1"), json!({"status": null, "records": {}, "reported": true}))));
        assert!(list[0].status.is_none(), "null clears it");
        // No `status` key, a bad shape or an unknown terminal change nothing.
        list[0].status = status("working", None);
        assert!(!apply(&mut list, &ev("status", Some("t1"), json!({}))));
        assert!(!apply(&mut list, &ev("status", Some("t1"), json!({"status": "nope"}))));
        assert!(!apply(&mut list, &ev("status", Some("t9"), json!({"status": null}))));
        assert_eq!(list[0].status.as_ref().map(|s| s.state.as_str()), Some("working"));
    }

    #[test]
    fn turn_size_and_foreground_set_their_field() {
        let mut list = vec![agent_t("t1", 1, "builder", "a")];
        assert!(apply(&mut list, &ev("turn", Some("t1"), json!({"turn": 5, "state": "idle"}))));
        assert_eq!(list[0].turn, 5);
        assert!(!apply(&mut list, &ev("turn", Some("t1"), json!({"state": "idle"}))));
        assert!(apply(&mut list, &ev("size", Some("t1"), json!({"cols": 90, "rows": 30}))));
        assert_eq!((list[0].cols, list[0].rows), (90, 30));
        assert!(!apply(&mut list, &ev("size", Some("t1"), json!({"cols": 90}))));
        assert!(apply(&mut list, &ev("foreground", Some("t1"), json!({"pid": 7, "name": "claude", "argv": [], "cwd": "/x"}))));
        assert_eq!(list[0].foreground.as_ref().map(|f| f.name.as_str()), Some("claude"));
        assert!(apply(&mut list, &ev("foreground", Some("t1"), Value::Null)));
        assert!(list[0].foreground.is_none());
        assert!(!apply(&mut list, &ev("foreground", Some("t9"), Value::Null)));
        assert!(!apply(&mut list, &ev("keep", Some("t1"), json!({"keep": true}))));
    }

    #[test]
    fn a_status_event_turns_the_agent_state() {
        let mut list = vec![agent_t("t1", 1, "orchestrator", "a"), agent_t("t2", 2, "builder", "a")];
        list[1].parent = Some("t1".into());
        let states = |l: &[Terminal]| run(l, 10).iter().map(|a| a.state.clone()).collect::<Vec<_>>();
        assert_eq!(states(&list), vec![AgentState::Idle, AgentState::Idle]);
        apply(&mut list, &ev("status", Some("t2"), json!({"status": {"state": "working"}})));
        assert_eq!(states(&list), vec![AgentState::Waiting, AgentState::Working]);
        apply(&mut list, &ev("status", Some("t2"), json!({"status": {"state": "blocked", "kind": "question", "msg": "Which?"}})));
        assert_eq!(states(&list), vec![AgentState::Idle, AgentState::NeedsYou { kind: Some("question".into()), msg: "Which?".into() }]);
        apply(&mut list, &ev("finished", Some("t2"), json!({"reason": "exited", "exit_code": 0})));
        assert_eq!(states(&list), vec![AgentState::Idle, AgentState::Ended]);
        // Done and error are rest states; an orchestrator that works itself is Working, not Waiting.
        apply(&mut list, &ev("status", Some("t1"), json!({"status": {"state": "working"}})));
        assert_eq!(states(&list)[0], AgentState::Working);
        apply(&mut list, &ev("status", Some("t1"), json!({"status": {"state": "done"}})));
        assert_eq!(states(&list)[0], AgentState::Idle);
    }

    #[test]
    fn an_orchestrator_waits_only_for_a_live_working_child() {
        let orch = agent_t("t1", 1, "orchestrator", "a");
        let mut kid = agent_t("t2", 2, "builder", "a");
        kid.parent = Some("t1".into());
        kid.status = status("working", None);
        // A working grandchild does not make the grandparent wait; a finished child does not either.
        let mut grand = agent_t("t3", 3, "builder", "a");
        grand.parent = Some("t2".into());
        grand.status = status("working", None);
        let mut idle_kid = kid.clone();
        idle_kid.status = None;
        assert_eq!(run(&[orch.clone(), idle_kid.clone(), grand], 10)[0].state, AgentState::Idle);
        // A blocked child needs the user: it does not make the orchestrator wait.
        let mut blocked_kid = kid.clone();
        blocked_kid.status = status("blocked", Some("question"));
        let a = run(&[orch.clone(), blocked_kid], 10);
        assert_eq!((&a[0].state, &a[1].state), (&AgentState::Idle, &AgentState::NeedsYou { kind: Some("question".into()), msg: "ok?".into() }));
        let mut done_kid = kid;
        done_kid.state = "finished".into();
        assert_eq!(run(&[orch, done_kid], 10)[0].state, AgentState::Idle);
    }

    #[test]
    fn only_activity_events_start_the_age_again() {
        for k in ["created", "restarted", "finished", "lost", "status", "turn", "title"] {
            assert!(is_activity(k), "{k}");
        }
        for k in ["size", "foreground", "labels", "keep", "removed", "doc"] {
            assert!(!is_activity(k), "{k}");
        }
    }

    fn live() -> Live {
        Live::new(vec![agent_t("t1", 1, "builder", "a"), agent_t("t2", 2, "builder", "a")], 5)
    }

    fn at(seq: u64, kind: &str, terminal: &str, data: Value) -> Event {
        Event { seq, kind: kind.into(), terminal: Some(terminal.into()), data }
    }

    #[test]
    fn live_drops_events_at_or_below_the_seen_seq() {
        let mut l = live();
        let working = json!({"status": {"state": "working"}});
        assert!(!l.event(&at(5, "status", "t1", working.clone()), 100), "the snapshot already holds seq 5");
        assert!(l.terminals[0].status.is_none() && l.stamps.is_empty());
        assert!(l.event(&at(6, "status", "t1", working.clone()), 100));
        assert!(!l.event(&at(6, "status", "t2", working.clone()), 200), "a repeat of seq 6");
        assert!(!l.event(&at(3, "status", "t2", working.clone()), 200), "an older one");
        assert!(l.terminals[1].status.is_none());
        assert!(l.event(&at(9, "status", "t2", working), 300), "gaps are fine");
        assert_eq!(l.stamps.get("t1"), Some(&100));
        assert_eq!(l.stamps.get("t2"), Some(&300));
    }

    #[test]
    fn live_stamps_activity_only_and_forgets_removed_terminals() {
        let mut l = live();
        assert!(l.event(&at(6, "size", "t1", json!({"cols": 1, "rows": 1})), 100));
        assert!(l.stamps.is_empty(), "a resize is not activity");
        assert!(l.event(&at(7, "turn", "t1", json!({"turn": 3})), 150));
        assert_eq!(l.stamps.get("t1"), Some(&150));
        assert!(!l.event(&at(8, "title", "t1", json!({})), 160), "nothing changed");
        assert_eq!(l.stamps.get("t1"), Some(&150));
        assert!(l.event(&at(9, "removed", "t1", json!({})), 170));
        assert!(!l.stamps.contains_key("t1") && l.terminals.len() == 1);
        let no_terminal = Event { seq: 10, kind: "doc".into(), terminal: None, data: json!({}) };
        assert!(!l.event(&no_terminal, 180));
    }

    #[test]
    fn a_missed_snapshot_replaces_the_list_and_sets_the_floor() {
        let mut l = live();
        l.event(&at(6, "turn", "t2", json!({"turn": 1})), 100);
        let mut a = agent_t("t1", 1, "builder", "a");
        a.status = status("blocked", Some("auth"));
        let fresh = vec![a, agent_t("t3", 3, "builder", "a")];
        assert!(l.missed(20, fresh.clone(), 500));
        assert_eq!(l.terminals, fresh, "t2 is gone, t3 is new");
        assert_eq!(l.stamps.get("t1"), Some(&500), "a changed record is active now");
        assert_eq!(l.stamps.get("t3"), Some(&500));
        assert!(!l.stamps.contains_key("t2"), "a gone terminal has no stamp");
        // Events behind the snapshot are history; the next one applies.
        assert!(!l.event(&at(15, "title", "t1", json!({"title": "old"})), 600));
        assert_eq!(l.terminals[0].title, "");
        assert!(l.event(&at(21, "title", "t1", json!({"title": "new"})), 700));
        // The same snapshot again changes nothing and stamps nothing new.
        let same = l.terminals.clone();
        assert!(!l.missed(30, same, 900));
        assert_eq!(l.stamps.get("t1"), Some(&700));
        // A stale snapshot never lowers the floor.
        l.missed(10, l.terminals.clone(), 950);
        assert!(!l.event(&at(25, "title", "t1", json!({"title": "x"})), 960));
    }
}
