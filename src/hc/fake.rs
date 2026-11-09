//! The in-process fake core (Contract C9): speaks the plain protocol from a fixture, so headless
//! runs and tests never dial a real hc. `CONTR_HC_FAKE_LOG=<file>` appends each mutating call as
//! a JSON line `{"method":…,"params":…}`.
//!
//! Fixture: `{"hello":{…},"status":{…},"terminals":[Terminal…],"events":[{"after_ms":300,
//! "method":"event","params":{…}}],"errors":{"<method>":{"kind":"blocked","message":"…"}}}`; every
//! key is optional. Framed mode, `terminal.run`/`attach` and screens come with H2.

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// Methods that change the core: logged, and not served by the fake yet.
const MUTATING: [&str; 4] = ["terminal.run", "terminal.send", "terminal.kill", "terminal.rm"];

/// How often the fake sends a heartbeat (the core sends one every 15 s).
const HEARTBEAT: Duration = Duration::from_secs(15);

// ───────────────────────────── in-memory pipe ─────────────────────────────

pub struct PipeReader {
    rx: Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

#[derive(Clone)]
pub struct PipeWriter {
    tx: Sender<Vec<u8>>,
}

/// A one-way byte pipe. Reading ends (`Ok(0)`) when every writer is dropped; writing fails with
/// `BrokenPipe` once the reader is dropped.
pub fn pipe() -> (PipeReader, PipeWriter) {
    let (tx, rx) = channel();
    (PipeReader { rx, buf: VecDeque::new() }, PipeWriter { tx })
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.buf.is_empty() {
            match self.rx.recv() {
                Ok(v) => self.buf.extend(v),
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len());
        for slot in out.iter_mut().take(n) {
            *slot = self.buf.pop_front().unwrap();
        }
        Ok(n)
    }
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx.send(data.to_vec()).map_err(|_| io::ErrorKind::BrokenPipe)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ───────────────────────────── fixture ─────────────────────────────

#[derive(Debug)]
pub struct Fixture {
    raw: Value,
}

impl Fixture {
    pub fn load(path: &Path) -> io::Result<Fixture> {
        let text = std::fs::read_to_string(path).map_err(|e| io::Error::new(e.kind(), format!("fake core fixture {}: {e}", path.display())))?;
        let raw = serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("fake core fixture {}: {e}", path.display())))?;
        Ok(Fixture::from_value(raw))
    }

    pub fn from_value(raw: Value) -> Fixture {
        Fixture { raw }
    }

    fn terminals(&self) -> Vec<Value> {
        self.raw["terminals"].as_array().cloned().unwrap_or_default()
    }

    fn hello(&self) -> Value {
        let mut hello = json!({
            "generation": 1, "version": "0.1.19", "methods": [], "features": [], "framed": false,
            "core": {"id": "fake", "name": "fake-core"}
        });
        merge(&mut hello, &self.raw["hello"]);
        hello
    }

    fn status(&self) -> Value {
        let hello = self.hello();
        let mut status = json!({
            "core": hello["core"], "version": hello["version"], "pid": 1, "uptime_ms": 0,
            "live": self.terminals().len(), "finished": 0, "holders": []
        });
        merge(&mut status, &self.raw["status"]);
        status
    }

    fn error_for(&self, method: &str) -> Option<Value> {
        let e = self.raw["errors"].get(method)?;
        let kind = e["kind"].as_str().unwrap_or("unavailable");
        let message = e["message"].as_str().unwrap_or("fake core error");
        Some(json!({"code": -32000, "message": message, "data": {"kind": kind, "hint": e["hint"]}}))
    }

    fn timed_events(&self) -> Vec<(u64, Value)> {
        let mut evs: Vec<(u64, Value)> = self.raw["events"]
            .as_array()
            .map(|a| a.iter().map(|e| (e["after_ms"].as_u64().unwrap_or(0), json!({"jsonrpc": "2.0", "method": e["method"], "params": e["params"]}))).collect())
            .unwrap_or_default();
        evs.sort_by_key(|(t, _)| *t);
        evs
    }
}

/// Copy the keys of object `over` onto `base`.
fn merge(base: &mut Value, over: &Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), over.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

// ───────────────────────────── server ─────────────────────────────

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

fn send(w: &SharedWriter, v: &Value) -> io::Result<()> {
    let mut line = v.to_string();
    line.push('\n');
    let mut w = w.lock().unwrap();
    w.write_all(line.as_bytes())?;
    w.flush()
}

fn log(path: &Option<String>, method: &str, params: &Value) {
    let Some(path) = path else { return };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", json!({"method": method, "params": params}));
    }
}

/// Serve one connection until the client hangs up. Blocks: run it on its own thread.
pub fn serve(fx: Arc<Fixture>, reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) {
    let log_path = std::env::var("CONTR_HC_FAKE_LOG").ok().filter(|p| !p.is_empty());
    serve_logged(fx, reader, writer, log_path);
}

/// `serve` with an explicit log file instead of `CONTR_HC_FAKE_LOG`.
pub fn serve_logged(fx: Arc<Fixture>, reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, log_path: Option<String>) {
    let writer: SharedWriter = Arc::new(Mutex::new(writer));
    let mut lines = BufReader::new(reader);
    let mut buf = String::new();
    loop {
        buf.clear();
        match lines.read_line(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Ok(req) = serde_json::from_str::<Value>(buf.trim()) else { continue };
        let (id, method, params) = (req["id"].clone(), req["method"].as_str().unwrap_or("").to_owned(), req["params"].clone());
        if MUTATING.contains(&method.as_str()) {
            log(&log_path, &method, &params);
        }
        let reply = match fx.error_for(&method) {
            Some(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
            None => match handle(&fx, &method, &params, &writer) {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err((kind, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message, "data": {"kind": kind, "hint": null}}}),
            },
        };
        if send(&writer, &reply).is_err() {
            return;
        }
    }
}

fn handle(fx: &Arc<Fixture>, method: &str, params: &Value, writer: &SharedWriter) -> Result<Value, (&'static str, String)> {
    match method {
        "hello" => Ok(fx.hello()),
        "server.status" => Ok(fx.status()),
        "terminal.list" => {
            let want = params["labels"].as_object();
            let terminals: Vec<Value> = fx
                .terminals()
                .into_iter()
                .filter(|t| want.is_none_or(|w| w.iter().all(|(k, v)| t["labels"].get(k) == Some(v))))
                .collect();
            Ok(json!({"terminals": terminals, "hidden_finished": 0, "machines": []}))
        }
        "terminal.get" => {
            let addr = params["terminal"].as_str().unwrap_or("");
            fx.terminals().into_iter().find(|t| t["address"] == addr).ok_or(("not_found", format!("no terminal \"{addr}\"")))
        }
        "events.subscribe" => {
            let (events, w) = (fx.timed_events(), writer.clone());
            let _ = std::thread::Builder::new().name("hc-fake-events".into()).spawn(move || {
                let mut at = 0;
                for (t, ev) in events {
                    std::thread::sleep(Duration::from_millis(t - at));
                    at = t;
                    if send(&w, &ev).is_err() {
                        return;
                    }
                }
                for seq in 1.. {
                    std::thread::sleep(HEARTBEAT);
                    if send(&w, &json!({"jsonrpc": "2.0", "method": "heartbeat", "params": {"seq": seq}})).is_err() {
                        return;
                    }
                }
            });
            Ok(json!({"seq": 0, "terminals": fx.terminals()}))
        }
        m if MUTATING.contains(&m) => Err(("unavailable", format!("the fake core does not serve {m} yet"))),
        m => Err(("unknown_method", format!("unknown method {m}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::conn::{Conn, HcError, Note};

    const T: Duration = Duration::from_secs(5);

    fn connect(fx: Value, notes: Option<Sender<Note>>) -> Conn {
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        let fx = Arc::new(Fixture::from_value(fx));
        std::thread::spawn(move || serve(fx, Box::new(req_r), Box::new(rep_w)));
        Conn::new(Box::new(rep_r), Box::new(req_w), notes)
    }

    #[test]
    fn the_pipe_reads_what_was_written_and_ends_when_the_writer_drops() {
        let (mut r, mut w) = pipe();
        w.write_all(b"hello").unwrap();
        drop(w);
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn hello_status_and_terminals_come_from_the_fixture() {
        let c = connect(json!({"hello": {"version": "9.9.9"}, "terminals": [{"address": "t1", "labels": {"role": "terminal"}}, {"address": "t2", "labels": {"role": "builder"}}]}), None);
        assert_eq!(c.call("hello", json!({"generation": 1}), T).unwrap()["version"], "9.9.9");
        assert_eq!(c.call("server.status", json!({}), T).unwrap()["version"], "9.9.9");
        assert_eq!(c.call("terminal.list", json!({"all": false}), T).unwrap()["terminals"].as_array().unwrap().len(), 2);
        let only = c.call("terminal.list", json!({"labels": {"role": "builder"}}), T).unwrap();
        assert_eq!(only["terminals"][0]["address"], "t2");
        assert_eq!(c.call("terminal.get", json!({"terminal": "t2"}), T).unwrap()["address"], "t2");
        assert!(matches!(c.call("terminal.get", json!({"terminal": "t9"}), T), Err(HcError::NotFound(_))));
    }

    #[test]
    fn errors_in_the_fixture_win_and_unknown_methods_fail() {
        let c = connect(json!({"errors": {"terminal.send": {"kind": "blocked", "message": "blocked"}}}), None);
        assert_eq!(c.call("terminal.send", json!({}), T).unwrap_err().kind(), Some("blocked"));
        assert_eq!(c.call("nope", json!({}), T).unwrap_err().kind(), Some("unknown_method"));
        assert_eq!(c.call("terminal.kill", json!({}), T).unwrap_err().kind(), Some("unavailable"));
    }

    #[test]
    fn mutating_calls_are_logged_and_reads_are_not() {
        let log = std::env::temp_dir().join(format!("contr-rune-fake-log-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&log);
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        let (fx, path) = (Arc::new(Fixture::from_value(json!({}))), Some(log.to_str().unwrap().to_owned()));
        std::thread::spawn(move || serve_logged(fx, Box::new(req_r), Box::new(rep_w), path));
        let c = Conn::new(Box::new(rep_r), Box::new(req_w), None);
        let _ = c.call("terminal.list", json!({}), T);
        let _ = c.call("terminal.send", json!({"terminal": "t3", "text": "hi\n"}), T);
        let text = std::fs::read_to_string(&log).unwrap();
        let _ = std::fs::remove_file(&log);
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines, vec![json!({"method": "terminal.send", "params": {"terminal": "t3", "text": "hi\n"}})]);
    }

    #[test]
    fn subscribe_returns_the_snapshot_then_sends_timed_events() {
        let (tx, rx) = channel();
        let c = connect(json!({"terminals": [{"address": "t1"}], "events": [{"after_ms": 20, "method": "event", "params": {"seq": 1, "type": "title", "terminal": "t1", "data": {"title": "x"}}}]}), Some(tx));
        let snap = c.call("events.subscribe", json!({}), T).unwrap();
        assert_eq!(snap["terminals"][0]["address"], "t1");
        let n = rx.recv_timeout(T).unwrap();
        assert_eq!((n.method.as_str(), n.params["type"].as_str()), ("event", Some("title")));
    }
}
