//! The plain hc connection: JSON-RPC 2.0, one message per line (Contract C1). A reader thread
//! (`hc-read`) routes replies by id to the waiting call and notifications to `notes`.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::wire::RpcError;

/// Why a call failed.
#[derive(Debug)]
pub enum HcError {
    /// The core said `not_found`.
    NotFound(String),
    /// The request was never written (closed connection): safe to retry.
    NotSent,
    /// The connection died while waiting: the call may have run, do not retry a `terminal.run`.
    Closed,
    /// Any other error object from the core.
    Rpc { kind: String, message: String, hint: Option<String> },
    /// Write failure, timeout or an unreadable reply.
    Io(io::Error),
}

impl HcError {
    pub fn from_rpc(e: RpcError) -> HcError {
        match e.kind() {
            "not_found" => HcError::NotFound(e.message),
            _ => HcError::Rpc { kind: e.kind().to_owned(), message: e.message, hint: e.data.and_then(|d| d.hint) },
        }
    }

    /// The error kind when the core sent one.
    pub fn kind(&self) -> Option<&str> {
        match self {
            HcError::NotFound(_) => Some("not_found"),
            HcError::Rpc { kind, .. } => Some(kind),
            _ => None,
        }
    }
}

impl fmt::Display for HcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HcError::NotFound(m) => write!(f, "{m}"),
            HcError::NotSent => write!(f, "connection closed"),
            HcError::Closed => write!(f, "connection lost"),
            HcError::Rpc { message, .. } => write!(f, "{message}"),
            HcError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A notification from the core (`event`, `heartbeat`, `missed`).
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub method: String,
    pub params: Value,
}

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, HcError>>>>>;

pub struct Conn {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl Conn {
    pub fn new(reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, notes: Option<Sender<Note>>) -> Conn {
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (p, c) = (pending.clone(), closed.clone());
        let spawned = std::thread::Builder::new().name("hc-read".into()).spawn(move || {
            read_loop(reader, &p, notes);
            // Mark closed first so a new call is `NotSent`, then drop the waiters: they see `Closed`.
            c.store(true, Ordering::Release);
            p.lock().unwrap().clear();
        });
        if spawned.is_err() {
            closed.store(true, Ordering::Release);
        }
        Conn { writer: Mutex::new(writer), pending, next_id: AtomicU64::new(1), closed }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Send `method` and wait for its reply.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HcError> {
        if self.is_closed() {
            return Err(HcError::NotSent);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);
        // The reader may have closed between the check and the insert: it clears `pending` after
        // setting `closed`, so check again to avoid waiting for a reply that cannot come.
        if self.is_closed() {
            self.pending.lock().unwrap().remove(&id);
            return Err(HcError::NotSent);
        }
        let mut line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        let sent = {
            let mut w = self.writer.lock().unwrap();
            w.write_all(line.as_bytes()).and_then(|()| w.flush())
        };
        if let Err(e) = sent {
            self.pending.lock().unwrap().remove(&id);
            return Err(if e.kind() == io::ErrorKind::BrokenPipe { HcError::NotSent } else { HcError::Io(e) });
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(HcError::Io(io::Error::new(io::ErrorKind::TimedOut, format!("{method}: no reply"))))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(HcError::Closed),
        }
    }
}

fn read_loop(reader: Box<dyn Read + Send>, pending: &Pending, notes: Option<Sender<Note>>) {
    let mut lines = BufReader::new(reader);
    let mut buf = String::new();
    loop {
        buf.clear();
        match lines.read_line(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Ok(v) = serde_json::from_str::<Value>(buf.trim()) else { continue };
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            let reply = match v.get("error") {
                Some(e) => Err(HcError::from_rpc(serde_json::from_value(e.clone()).unwrap_or_default())),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let _ = tx.send(reply);
            }
        } else if let Some(method) = v.get("method").and_then(Value::as_str) {
            if let Some(n) = &notes {
                let _ = n.send(Note { method: method.to_owned(), params: v.get("params").cloned().unwrap_or(Value::Null) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::fake::pipe;
    use std::io::BufRead;
    use std::sync::mpsc::channel;

    const T: Duration = Duration::from_secs(5);

    /// A conn wired to a scripted "core": returns the conn, the core's reader of requests and its writer.
    fn rig(notes: Option<Sender<Note>>) -> (Conn, BufReader<crate::hc::fake::PipeReader>, crate::hc::fake::PipeWriter) {
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        (Conn::new(Box::new(rep_r), Box::new(req_w), notes), BufReader::new(req_r), rep_w)
    }

    fn request(core: &mut BufReader<crate::hc::fake::PipeReader>) -> Value {
        let mut l = String::new();
        core.read_line(&mut l).unwrap();
        serde_json::from_str(&l).unwrap()
    }

    #[test]
    fn replies_are_routed_by_id_even_out_of_order() {
        let (conn, mut core, mut out) = rig(None);
        let conn = Arc::new(conn);
        let (c1, c2) = (conn.clone(), conn.clone());
        let a = std::thread::spawn(move || c1.call("a", json!({}), T));
        let r1 = request(&mut core);
        let b = std::thread::spawn(move || c2.call("b", json!({}), T));
        let r2 = request(&mut core);
        assert_eq!((r1["method"].as_str(), r2["method"].as_str()), (Some("a"), Some("b")));
        // Answer the second request first.
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": r2["id"], "result": "two"})).unwrap();
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": r1["id"], "result": "one"})).unwrap();
        assert_eq!(a.join().unwrap().unwrap(), json!("one"));
        assert_eq!(b.join().unwrap().unwrap(), json!("two"));
    }

    #[test]
    fn notifications_go_to_the_notes_channel_not_to_calls() {
        let (tx, rx) = channel();
        let (conn, mut core, mut out) = rig(Some(tx));
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "method": "heartbeat", "params": {"seq": 4}})).unwrap();
        let note = rx.recv_timeout(T).unwrap();
        assert_eq!(note, Note { method: "heartbeat".into(), params: json!({"seq": 4}) });
        let h = std::thread::spawn(move || conn.call("x", json!({}), T));
        let r = request(&mut core);
        writeln!(out, "{}", json!({"id": r["id"], "result": 1})).unwrap();
        assert_eq!(h.join().unwrap().unwrap(), json!(1));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn an_error_object_keeps_its_kind_and_not_found_has_its_own_variant() {
        let (conn, mut core, mut out) = rig(None);
        let conn = Arc::new(conn);
        let c = conn.clone();
        let h = std::thread::spawn(move || c.call("terminal.get", json!({"terminal": "t9"}), T));
        let r = request(&mut core);
        let err = json!({"code": -32000, "message": "no terminal \"t9\"", "data": {"kind": "not_found", "hint": "see ls"}});
        writeln!(out, "{}", json!({"id": r["id"], "error": err})).unwrap();
        match h.join().unwrap() {
            Err(HcError::NotFound(m)) => assert!(m.contains("t9")),
            other => panic!("{other:?}"),
        }
        let c = conn.clone();
        let h = std::thread::spawn(move || c.call("terminal.send", json!({}), T));
        let r = request(&mut core);
        let err = json!({"code": -32000, "message": "agent is blocked", "data": {"kind": "blocked", "hint": "answer it"}});
        writeln!(out, "{}", json!({"id": r["id"], "error": err})).unwrap();
        match h.join().unwrap() {
            Err(HcError::Rpc { kind, message, hint }) => {
                assert_eq!((kind.as_str(), message.as_str(), hint.as_deref()), ("blocked", "agent is blocked", Some("answer it")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_call_on_a_closed_connection_was_not_sent() {
        let (conn, _core, out) = rig(None);
        drop(out); // the core hangs up
        for _ in 0..200 {
            if conn.is_closed() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(conn.call("x", json!({}), T), Err(HcError::NotSent)));
    }

    #[test]
    fn a_connection_that_dies_while_waiting_is_closed_not_retried() {
        let (conn, mut core, out) = rig(None);
        let h = std::thread::spawn(move || conn.call("terminal.run", json!({}), T));
        request(&mut core);
        drop(out);
        assert!(matches!(h.join().unwrap(), Err(HcError::Closed)));
    }

    #[test]
    fn no_reply_in_time_is_a_timeout() {
        let (conn, _core, _out) = rig(None);
        match conn.call("x", json!({}), Duration::from_millis(50)) {
            Err(HcError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::TimedOut),
            other => panic!("{other:?}"),
        }
    }
}
