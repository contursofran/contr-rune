//! The hc watcher (`hc-watch` thread): dial, `hello`, `server.status`, then a second connection
//! with `events.subscribe` to notice when the core goes away. It reports the connection state to
//! the UI over an mpsc channel and wakes the window. Reconnect: 0.5 s doubled to 5 s.
//!
//! H1 keeps the state; H4 also forwards the events that matter to tabs (`restarted`, `finished`,
//! `removed`). H5 keeps the terminal list and H6 keeps it live: the snapshot of `events.subscribe`,
//! then every event (`model::Live`: `seq` order, `missed` snapshots) with the time of each
//! terminal's last activity, sent after every change.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rune::Waker;
use serde_json::json;

use super::conn::{Conn, Note};
use super::dial::{Dialer, Keep};
use super::model;
use super::wire::{Event, HelloResult, Missed, ServerStatus, Snapshot, Terminal};

const CALL: Duration = Duration::from_secs(10);
/// The core sends a heartbeat every 15 s; three missed ones mean the line is dead.
const SILENCE: Duration = Duration::from_secs(45);
const BACKOFF_FIRST: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// Longest error text shown in Settings.
const LABEL_MAX: usize = 64;

/// What Settings shows about hc.
#[derive(Clone, Debug, PartialEq)]
pub enum HcStatus {
    /// `CONTR_HC=off`.
    Off,
    NotInstalled,
    Connecting,
    Connected { version: String },
    Failed(String),
}

impl HcStatus {
    /// The Settings value: the version, "not installed", "connecting", "off" or "not running: <error>".
    pub fn label(&self) -> String {
        match self {
            HcStatus::Off => "off".to_owned(),
            HcStatus::NotInstalled => "not installed".to_owned(),
            HcStatus::Connecting => "connecting".to_owned(),
            HcStatus::Connected { version } => version.clone(),
            HcStatus::Failed(e) => {
                let e = e.lines().next().unwrap_or("").trim();
                let short: String = e.chars().take(LABEL_MAX).collect();
                format!("not running: {short}{}", if e.chars().count() > LABEL_MAX { "…" } else { "" })
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, HcStatus::Connected { .. })
    }
}

/// The events a tab reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// The terminal got a new shell: attach again.
    Restarted,
    /// The program ended (the tab closes).
    Finished,
    /// The record is gone (the tab closes).
    Removed,
}

/// Messages from the background threads to the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum HcUpdate {
    Status(HcStatus),
    /// An event for one terminal.
    Event { terminal: String, kind: TermEvent },
    /// The launch listing of this app's tabs (`restore::list`).
    Restore(Result<Vec<Terminal>, String>),
    /// The terminals the core has now, with the time (ms) of each one's last event.
    Terminals { terminals: Vec<Terminal>, stamps: HashMap<String, u64> },
}

/// The update a notification makes, if any.
fn to_update(note: &Note) -> Option<HcUpdate> {
    if note.method != "event" {
        return None;
    }
    let ev: Event = serde_json::from_value(note.params.clone()).ok()?;
    let kind = match ev.kind.as_str() {
        "restarted" => TermEvent::Restarted,
        "finished" => TermEvent::Finished,
        "removed" => TermEvent::Removed,
        _ => return None,
    };
    Some(HcUpdate::Event { terminal: ev.terminal?, kind })
}

/// A line to the core with its plain connection.
struct Opened {
    conn: Conn,
    keep: Keep,
}

fn open(d: &Dialer, notes: Option<Sender<Note>>) -> Result<Opened, String> {
    let link = d.dial().map_err(|e| e.to_string())?;
    Ok(Opened { conn: Conn::new(link.reader, link.writer, notes), keep: link.keep })
}

impl Opened {
    /// An error text, with what `hc connect` said on stderr when it has something to add.
    fn explain(&self, e: impl std::fmt::Display) -> String {
        match self.keep.stderr_tail() {
            Some(t) => format!("{e}: {t}"),
            None => e.to_string(),
        }
    }
}

/// `hello` with generation 1; refuses any other generation.
pub fn handshake(conn: &Conn, framed: bool) -> Result<HelloResult, String> {
    let params = if framed { json!({"generation": 1, "framed": true}) } else { json!({"generation": 1}) };
    let v = conn.call("hello", params, CALL).map_err(|e| e.to_string())?;
    let hello: HelloResult = serde_json::from_value(v).map_err(|e| format!("bad hello reply: {e}"))?;
    if hello.generation != 1 {
        return Err(format!("unsupported hc protocol generation {} (need 1)", hello.generation));
    }
    Ok(hello)
}

/// Dial, `hello`, `server.status`: the connected version, or why not.
fn check(d: &Dialer) -> Result<(Opened, String), String> {
    let opened = open(d, None)?;
    let hello = handshake(&opened.conn, false).map_err(|e| opened.explain(e))?;
    let status: ServerStatus = opened
        .conn
        .call("server.status", json!({}), CALL)
        .map_err(|e| opened.explain(e))
        .and_then(|v| serde_json::from_value(v).map_err(|e| format!("bad server.status reply: {e}")))?;
    let version = if status.version.is_empty() { hello.version } else { status.version };
    Ok((opened, version))
}

/// One synchronous check, for headless runs (a scene cannot wait for the thread).
pub fn probe(d: &Dialer) -> HcStatus {
    match check(d) {
        Ok((_, version)) => HcStatus::Connected { version },
        Err(e) => HcStatus::Failed(e),
    }
}

/// Start the watcher thread.
pub fn spawn(d: Dialer, tx: Sender<HcUpdate>, wake: Waker) {
    spawn_with(d, tx, wake, true);
}

/// The watcher for a headless run: it forwards events but leaves the status to `probe`.
pub fn spawn_events(d: Dialer, tx: Sender<HcUpdate>, wake: Waker) {
    spawn_with(d, tx, wake, false);
}

fn spawn_with(d: Dialer, tx: Sender<HcUpdate>, wake: Waker, report: bool) {
    let send = move |u: HcUpdate| {
        let ok = tx.send(u).is_ok();
        wake();
        ok
    };
    let _ = std::thread::Builder::new().name("hc-watch".into()).spawn(move || run(&d, &send, report));
}

fn run(d: &Dialer, send: &dyn Fn(HcUpdate) -> bool, report: bool) {
    let publish = |s: HcStatus| !report || send(HcUpdate::Status(s));
    let mut backoff = BACKOFF_FIRST;
    loop {
        if !publish(HcStatus::Connecting) {
            return;
        }
        let (was_connected, why) = hold(d, &publish, send);
        if was_connected {
            backoff = BACKOFF_FIRST;
            if !publish(HcStatus::Connecting) {
                return;
            }
        } else {
            if !publish(HcStatus::Failed(why)) {
                return;
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
        std::thread::sleep(backoff);
    }
}

/// Connect, publish `Connected`, and stay until the line is lost. Returns whether it connected,
/// and why it ended.
fn hold(d: &Dialer, publish: &dyn Fn(HcStatus) -> bool, send: &dyn Fn(HcUpdate) -> bool) -> (bool, String) {
    let (_main, version) = match check(d) {
        Ok(ok) => ok,
        Err(e) => return (false, e),
    };
    // The events line first, so a core that cannot stream is not reported as connected.
    let (note_tx, note_rx) = channel();
    let (events, snapshot) = match subscribe(d, note_tx) {
        Ok(o) => o,
        Err(e) => return (false, e),
    };
    if !publish(HcStatus::Connected { version }) {
        return (true, String::new());
    }
    forward_until_loss(&note_rx, send, snapshot.terminals, snapshot.seq);
    drop(events);
    (true, "connection lost".to_owned())
}

fn subscribe(d: &Dialer, notes: Sender<Note>) -> Result<(Opened, Snapshot), String> {
    let o = open(d, Some(notes))?;
    handshake(&o.conn, false).map_err(|e| o.explain(e))?;
    let snap = o.conn.call("events.subscribe", json!({}), CALL).map_err(|e| o.explain(e))?;
    let snap = serde_json::from_value::<Snapshot>(snap).map_err(|e| format!("bad events.subscribe reply: {e}"))?;
    Ok((o, snap))
}

/// Keep the terminal list current and send it (and the events tabs care about) until the events
/// line ends or goes silent. `seq` is the snapshot's: events at or below it are history.
fn forward_until_loss(rx: &Receiver<Note>, send: &dyn Fn(HcUpdate) -> bool, terminals: Vec<Terminal>, seq: u64) {
    let mut live = model::Live::new(terminals, seq);
    if !send(HcUpdate::Terminals { terminals: live.terminals.clone(), stamps: live.stamps.clone() }) {
        return;
    }
    while let Ok(note) = rx.recv_timeout(SILENCE) {
        let now = crate::session::now_ms();
        let changed = match note.method.as_str() {
            "event" => {
                if let Some(u) = to_update(&note)
                    && !send(u)
                {
                    return;
                }
                match serde_json::from_value::<Event>(note.params.clone()) {
                    Ok(ev) => live.event(&ev, now),
                    Err(_) => false,
                }
            }
            "missed" => match serde_json::from_value::<Missed>(note.params.clone()) {
                Ok(m) => live.missed(m.seq, m.terminals, now),
                Err(_) => false,
            },
            _ => false,
        };
        if changed && !send(HcUpdate::Terminals { terminals: live.terminals.clone(), stamps: live.stamps.clone() }) {
            return;
        }
    }
}

/// Every terminal the core has (`terminal.list`), on a short-lived plain connection: the first
/// listing of a headless run, which cannot wait for the watcher thread.
pub fn list_all(d: &Dialer) -> Result<Vec<Terminal>, String> {
    let link = d.dial().map_err(|e| e.to_string())?;
    let conn = Conn::new(link.reader, link.writer, None);
    handshake(&conn, false)?;
    let v = conn.call("terminal.list", json!({"all": false}), CALL).map_err(|e| e.to_string())?;
    let list: super::wire::TerminalList = serde_json::from_value(v).map_err(|e| format!("bad terminal.list reply: {e}"))?;
    drop(link.keep);
    Ok(list.terminals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::fake::Fixture;
    use serde_json::Value;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fake(fx: Value) -> Dialer {
        Dialer::Fake(Arc::new(Fixture::from_value(fx)))
    }

    #[test]
    fn labels_for_every_state() {
        assert_eq!(HcStatus::Off.label(), "off");
        assert_eq!(HcStatus::NotInstalled.label(), "not installed");
        assert_eq!(HcStatus::Connecting.label(), "connecting");
        assert_eq!(HcStatus::Connected { version: "0.1.19".into() }.label(), "0.1.19");
        assert_eq!(HcStatus::Failed("no socket\nsecond line".into()).label(), "not running: no socket");
        let long = HcStatus::Failed("x".repeat(200)).label();
        assert!(long.starts_with("not running: xxx") && long.ends_with('…') && long.chars().count() < 90, "{long}");
    }

    #[test]
    fn the_handshake_returns_the_hello() {
        let (_c, h) = {
            let o = open(&fake(json!({"hello": {"version": "0.1.19"}})), None).unwrap();
            let h = handshake(&o.conn, false).unwrap();
            (o, h)
        };
        assert_eq!((h.generation, h.version.as_str()), (1, "0.1.19"));
    }

    #[test]
    fn the_handshake_refuses_another_generation() {
        let o = open(&fake(json!({"hello": {"generation": 2}})), None).unwrap();
        let e = handshake(&o.conn, false).unwrap_err();
        assert!(e.contains("generation 2"), "{e}");
    }

    #[test]
    fn probe_reads_the_status_version_and_reports_failures() {
        assert_eq!(probe(&fake(json!({}))), HcStatus::Connected { version: "0.1.19".into() });
        assert_eq!(probe(&fake(json!({"status": {"version": "0.2.0"}}))), HcStatus::Connected { version: "0.2.0".into() });
        match probe(&fake(json!({"errors": {"server.status": {"kind": "unavailable", "message": "core is starting"}}}))) {
            HcStatus::Failed(m) => assert!(m.contains("core is starting"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(probe(&fake(json!({"hello": {"generation": 3}}))), HcStatus::Failed(_)));
    }

    #[test]
    fn a_missing_program_is_a_failure_with_the_os_error() {
        let d = Dialer::Command { path: "/nonexistent/contr-rune-hc".into() };
        assert!(matches!(probe(&d), HcStatus::Failed(m) if !m.is_empty()));
    }

    #[test]
    fn the_watcher_publishes_connecting_then_connected() {
        let (tx, rx) = channel();
        let woke = Arc::new(AtomicUsize::new(0));
        let w = woke.clone();
        spawn(fake(json!({})), tx, Arc::new(move || { w.fetch_add(1, Ordering::Relaxed); }));
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first, HcUpdate::Status(HcStatus::Connecting));
        let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(second, HcUpdate::Status(HcStatus::Connected { version: "0.1.19".into() }));
        assert!(woke.load(Ordering::Relaxed) >= 2, "the window is woken per update");
    }

    #[test]
    fn only_tab_events_are_forwarded() {
        let note = |ty: &str, terminal: Option<&str>| Note { method: "event".into(), params: json!({"seq": 1, "type": ty, "terminal": terminal, "data": {}}) };
        assert_eq!(to_update(&note("restarted", Some("t7"))), Some(HcUpdate::Event { terminal: "t7".into(), kind: TermEvent::Restarted }));
        assert_eq!(to_update(&note("finished", Some("t7"))), Some(HcUpdate::Event { terminal: "t7".into(), kind: TermEvent::Finished }));
        assert_eq!(to_update(&note("removed", Some("t8"))), Some(HcUpdate::Event { terminal: "t8".into(), kind: TermEvent::Removed }));
        assert_eq!(to_update(&note("status", Some("t7"))), None);
        assert_eq!(to_update(&note("doc", None)), None);
        assert_eq!(to_update(&note("finished", None)), None);
        assert_eq!(to_update(&Note { method: "heartbeat".into(), params: json!({"seq": 2}) }), None);
    }

    #[test]
    fn the_headless_watcher_forwards_fixture_events_without_status() {
        let ev = json!({"after_ms": 50, "method": "event", "params": {"seq": 1, "type": "finished", "terminal": "t3", "data": {"reason": "exited"}}});
        let (tx, rx) = channel();
        spawn_events(fake(json!({"events": [ev]})), tx, Arc::new(|| {}));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), HcUpdate::Terminals { .. }), "the snapshot comes first");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), HcUpdate::Event { terminal: "t3".into(), kind: TermEvent::Finished });
    }

    #[test]
    fn the_watcher_keeps_the_terminal_list_with_event_stamps() {
        let rec = |a: &str| json!({"address": a, "state": "live", "labels": {}});
        let ev = |ms: u64, seq: u64, ty: &str, t: &str, data: Value| json!({"after_ms": ms, "method": "event", "params": {"seq": seq, "type": ty, "terminal": t, "data": data}});
        let fx = json!({"terminals": [rec("t1")], "events": [
            ev(50, 1, "created", "t2", rec("t2")),
            ev(100, 2, "removed", "t1", json!({})),
        ]});
        let (tx, rx) = channel();
        spawn_events(fake(fx), tx, Arc::new(|| {}));
        let mut lists = Vec::new();
        while lists.len() < 3 {
            if let HcUpdate::Terminals { terminals, stamps } = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                lists.push((terminals.iter().map(|t| t.address.clone()).collect::<Vec<_>>(), stamps));
            }
        }
        assert_eq!(lists[0].0, vec!["t1"]);
        assert!(lists[0].1.is_empty());
        assert_eq!(lists[1].0, vec!["t1", "t2"]);
        assert!(lists[1].1["t2"] > 0);
        assert_eq!(lists[2].0, vec!["t2"]);
        assert!(!lists[2].1.contains_key("t1"), "a removed terminal has no stamp");
    }

    #[test]
    fn list_all_reads_every_terminal() {
        let fx = json!({"terminals": [{"address": "t1"}, {"address": "t2"}]});
        let all = list_all(&fake(fx)).unwrap();
        assert_eq!(all.iter().map(|t| t.address.as_str()).collect::<Vec<_>>(), vec!["t1", "t2"]);
    }

    #[test]
    fn the_watcher_reports_failed_when_events_cannot_subscribe() {
        let (tx, rx) = channel();
        spawn(fake(json!({"errors": {"events.subscribe": {"kind": "unavailable", "message": "no events"}}})), tx, Arc::new(|| {}));
        let mut last = None;
        for _ in 0..2 {
            last = Some(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        match last.unwrap() {
            HcUpdate::Status(HcStatus::Failed(m)) => assert!(m.contains("no events"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
}
