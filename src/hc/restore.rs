//! Reopening the app's hc terminals at launch (H4): which live terminals are tabs of this app, which
//! project each belongs to, and in what order they open. Pure; the listing is one short call.
//!
//! A terminal is a tab when it is live and carries `contr=agent` and `role=terminal` (Contract C5).
//! Its folder is the `worktree` label (else its `cwd`); the project is the known one with that
//! folder. A folder that is not a project yet is added when it exists and skipped when it does not.
//! Without any folder the `project` label is matched against the project names. Order: `created_ms`,
//! then the number in the address, so the tabs come back as they were opened.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use rune::Waker;
use serde_json::json;

use super::conn::Conn;
use super::dial::Dialer;
use super::watch::{self, HcUpdate};
use super::wire::{Terminal, TerminalList};

const CALL: Duration = Duration::from_secs(10);

/// A project the app already has.
pub struct Known<'a> {
    pub name: &'a str,
    pub path: &'a Path,
}

#[derive(Debug, PartialEq)]
pub enum Target {
    /// Index into the known projects.
    Project(usize),
    /// A folder that is no project yet but exists: add it.
    Folder(PathBuf),
}

#[derive(Debug, PartialEq)]
pub struct Item {
    pub address: String,
    pub target: Target,
}

/// Whether `t` is one of this app's tabs.
pub fn is_tab(t: &Terminal) -> bool {
    t.state != "finished" && t.labels.get("contr").is_some_and(|v| v == "agent") && t.labels.get("role").is_some_and(|v| v == "terminal")
}

/// The terminal's start folder: label `worktree`, else `cwd`.
fn folder(t: &Terminal) -> Option<PathBuf> {
    let w = t.labels.get("worktree").map(String::as_str).filter(|w| !w.is_empty());
    w.or(Some(t.cwd.as_str()).filter(|c| !c.is_empty())).map(PathBuf::from)
}

fn number(address: &str) -> u64 {
    address.trim_start_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(u64::MAX)
}

/// The tabs to reopen, in order. `same` compares two folders (symlinks resolved); `exists` tells a
/// folder that is still there.
pub fn plan(terminals: &[Terminal], known: &[Known], same: &dyn Fn(&Path, &Path) -> bool, exists: &dyn Fn(&Path) -> bool) -> Vec<Item> {
    let mut tabs: Vec<&Terminal> = terminals.iter().filter(|t| is_tab(t)).collect();
    tabs.sort_by_key(|t| (t.created_ms, number(&t.address)));
    let mut out = Vec::new();
    for t in tabs {
        let target = match folder(t) {
            Some(p) => match known.iter().position(|k| same(k.path, &p)) {
                Some(i) => Target::Project(i),
                None if exists(&p) => Target::Folder(p),
                None => continue,
            },
            None => match t.labels.get("project").and_then(|n| known.iter().position(|k| k.name == n)) {
                Some(i) => Target::Project(i),
                None => continue,
            },
        };
        out.push(Item { address: t.address.clone(), target });
    }
    out
}

/// `terminal.list` for this app's tabs, on a short-lived plain connection.
pub fn list(d: &Dialer) -> Result<Vec<Terminal>, String> {
    let link = d.dial().map_err(|e| e.to_string())?;
    let conn = Conn::new(link.reader, link.writer, None);
    watch::handshake(&conn, false)?;
    let v = conn.call("terminal.list", json!({"all": false, "labels": {"contr": "agent", "role": "terminal"}}), CALL).map_err(|e| e.to_string())?;
    let list: TerminalList = serde_json::from_value(v).map_err(|e| format!("bad terminal.list reply: {e}"))?;
    drop(link.keep);
    Ok(list.terminals)
}

/// `list` on a thread; the answer comes back as `HcUpdate::Restore`.
pub fn spawn(d: Dialer, tx: Sender<HcUpdate>, wake: Waker) {
    let _ = std::thread::Builder::new().name("hc-restore".into()).spawn(move || {
        let _ = tx.send(HcUpdate::Restore(list(&d)));
        wake();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(address: &str, created: u64, labels: &[(&str, &str)]) -> Terminal {
        let mut t = Terminal { address: address.into(), state: "live".into(), created_ms: created, ..Terminal::default() };
        for (k, v) in labels {
            t.labels.insert((*k).into(), (*v).into());
        }
        t
    }

    fn tab(address: &str, created: u64, project: &str, worktree: &str) -> Terminal {
        term(address, created, &[("contr", "agent"), ("role", "terminal"), ("project", project), ("worktree", worktree)])
    }

    fn run(terminals: &[Terminal], known: &[(&str, &str)], existing: &[&str]) -> Vec<Item> {
        let known: Vec<Known> = known.iter().map(|(n, p)| Known { name: n, path: Path::new(p) }).collect();
        plan(terminals, &known, &|a, b| a == b, &|p| existing.iter().any(|e| Path::new(e) == p))
    }

    fn addrs(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.address.as_str()).collect()
    }

    #[test]
    fn tabs_of_known_projects_match_by_folder() {
        let ts = [tab("t1", 1, "a", "/p/a"), tab("t2", 2, "b", "/p/b")];
        let items = run(&ts, &[("b", "/p/b"), ("a", "/p/a")], &[]);
        assert_eq!(items, vec![Item { address: "t1".into(), target: Target::Project(1) }, Item { address: "t2".into(), target: Target::Project(0) }]);
    }

    #[test]
    fn the_order_is_creation_time_then_address_number() {
        let ts = [tab("t10", 5, "a", "/p/a"), tab("t9", 5, "a", "/p/a"), tab("t30", 1, "a", "/p/a"), tab("t2", 9, "a", "/p/a")];
        assert_eq!(addrs(&run(&ts, &[("a", "/p/a")], &[])), vec!["t30", "t9", "t10", "t2"]);
    }

    #[test]
    fn foreign_finished_and_other_roles_are_not_tabs() {
        let mut finished = tab("t1", 1, "a", "/p/a");
        finished.state = "finished".into();
        let ts = [
            finished,
            term("t2", 2, &[("role", "terminal"), ("worktree", "/p/a")]),
            term("t3", 3, &[("contr", "agent"), ("role", "builder"), ("worktree", "/p/a")]),
            term("t4", 4, &[("contr", "agent"), ("role", "terminal"), ("worktree", "/p/a")]),
        ];
        assert_eq!(addrs(&run(&ts, &[("a", "/p/a")], &[])), vec!["t4"]);
    }

    #[test]
    fn an_unknown_folder_is_added_when_it_exists_and_skipped_when_not() {
        let ts = [tab("t1", 1, "x", "/p/x"), tab("t2", 2, "y", "/gone/y"), tab("t3", 3, "x", "/p/x")];
        let items = run(&ts, &[("a", "/p/a")], &["/p/x"]);
        assert_eq!(items, vec![Item { address: "t1".into(), target: Target::Folder("/p/x".into()) }, Item { address: "t3".into(), target: Target::Folder("/p/x".into()) }]);
    }

    #[test]
    fn without_a_folder_the_project_label_is_matched_by_name() {
        let ts = [term("t1", 1, &[("contr", "agent"), ("role", "terminal"), ("project", "a")]), term("t2", 2, &[("contr", "agent"), ("role", "terminal"), ("project", "zzz")])];
        assert_eq!(run(&ts, &[("a", "/p/a")], &[]), vec![Item { address: "t1".into(), target: Target::Project(0) }]);
    }

    #[test]
    fn the_cwd_stands_in_for_a_missing_worktree_label() {
        let mut t = term("t1", 1, &[("contr", "agent"), ("role", "terminal")]);
        t.cwd = "/p/a".into();
        assert_eq!(run(&[t], &[("a", "/p/a")], &[]), vec![Item { address: "t1".into(), target: Target::Project(0) }]);
    }

    #[test]
    fn no_terminals_no_items() {
        assert!(run(&[], &[("a", "/p/a")], &[]).is_empty());
    }
}
