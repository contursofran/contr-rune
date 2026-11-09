//! The in-process fake core (Contract C9): speaks the plain and the framed protocol from a
//! fixture, so headless runs and tests never dial a real hc.
//!
//! `CONTR_HC_FAKE_LOG=<file>` appends each mutating call as a JSON line `{"method":…,"params":…}`
//! and each stream input frame as `{"frame":"data","terminal":…,"bytes":"<utf8>"}`,
//! `{"frame":"resize","terminal":…,"cols":…,"rows":…}` or `{"frame":"close","terminal":…}`.
//!
//! Fixture: `{"hello":{…},"status":{…},"terminals":[Terminal…],"screens":{"t3":"<first paint>"},
//! "events":[{"after_ms":300,"method":"event","params":{…}}],"errors":{"<method>":{"kind":
//! "blocked","message":"…"}},"echo":true}`; every key is optional.
//!
//! Framed mode: `hello {"framed":true}` switches the connection after the reply. Then
//! `terminal.attach` answers a channel and streams the terminal's screen (first paint, then
//! `ESC [ ? 9001 l`) honouring CREDIT (a stream starts with 256 KiB); with `echo`, DATA input is
//! written back as output to every viewer of the terminal. `terminal.kill` closes the viewers
//! with `{"reason":"killed",…}`; a viewer CLOSE is answered with an empty CLOSE. Not served:
//! `terminal.send` (answers `unavailable`, or the fixture's error), `automatic_input`,
//! `resize_notifications` and FACTS.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::framed::{Decoder, Kind, SendWindow, encode, parse_credit, parse_resize};

/// Methods that change the core: logged.
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

// ───────────────────────────── fixture and core state ─────────────────────────────

/// Largest DATA payload the fake sends in one frame.
const CHUNK: usize = 16 * 1024;

pub struct Fixture {
    raw: Value,
    core: Mutex<Core>,
}

/// What `terminal.run/kill/rm` change; shared by every connection to the fixture.
struct Core {
    terminals: Vec<Value>,
    screens: HashMap<String, String>,
    next: u64,
    streams: HashMap<String, Vec<Arc<Stream>>>,
}

impl fmt::Debug for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fixture").finish_non_exhaustive()
    }
}

impl Fixture {
    pub fn load(path: &Path) -> io::Result<Fixture> {
        let text = std::fs::read_to_string(path).map_err(|e| io::Error::new(e.kind(), format!("fake core fixture {}: {e}", path.display())))?;
        let raw = serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("fake core fixture {}: {e}", path.display())))?;
        Ok(Fixture::from_value(raw))
    }

    pub fn from_value(raw: Value) -> Fixture {
        let terminals: Vec<Value> = raw["terminals"].as_array().cloned().unwrap_or_default();
        let next = terminals.iter().filter_map(|t| t["address"].as_str()?.strip_prefix('t')?.parse::<u64>().ok()).max().map_or(1, |n| n + 1);
        let screens = raw["screens"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()).unwrap_or_default();
        Fixture { core: Mutex::new(Core { terminals, screens, next, streams: HashMap::new() }), raw }
    }

    fn terminals(&self) -> Vec<Value> {
        self.core.lock().unwrap().terminals.clone()
    }

    fn echo(&self) -> bool {
        self.raw["echo"].as_bool().unwrap_or(false)
    }

    fn hello(&self, framed: bool) -> Value {
        let mut hello = json!({
            "generation": 1, "version": "0.1.19", "methods": [], "features": [], "framed": false,
            "core": {"id": "fake", "name": "fake-core"}
        });
        merge(&mut hello, &self.raw["hello"]);
        if framed {
            hello["framed"] = json!(true);
        }
        hello
    }

    fn status(&self) -> Value {
        let hello = self.hello(false);
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

    /// `terminal.run`: a new live terminal `t<next>` with the given labels and the screen `"$ "`.
    fn run(&self, params: &Value) -> Result<Value, Fail> {
        let mut core = self.core.lock().unwrap();
        let name = params["name"].as_str().map(str::to_owned);
        if let Some(n) = &name {
            if core.terminals.iter().any(|t| t["name"] == n.as_str() && t["state"] != "finished") {
                return Err(("name_taken", format!("terminal name \"{n}\" is taken")));
            }
        }
        let address = format!("t{}", core.next);
        core.next += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let rec = json!({
            "address": address, "name": name.unwrap_or_else(|| address.clone()), "parent": null,
            "cwd": params["cwd"], "argv": params.get("argv").cloned().unwrap_or(json!([])),
            "labels": params.get("labels").cloned().unwrap_or(json!({})),
            "state": "live", "turn": 0, "title": "", "exit_code": null, "reason": null,
            "created_ms": now, "status_ms": now,
            "cols": params["size"]["cols"].as_u64().unwrap_or(80), "rows": params["size"]["rows"].as_u64().unwrap_or(24),
            "foreground": null, "status": null
        });
        core.terminals.push(rec.clone());
        core.screens.insert(address, "$ ".to_owned());
        let mut out = rec;
        out["output"] = json!("");
        out["wait"] = Value::Null;
        Ok(out)
    }

    fn get(&self, address: &str) -> Result<Value, Fail> {
        self.terminals().into_iter().find(|t| t["address"] == address).ok_or(("not_found", format!("no terminal \"{address}\"")))
    }

    /// `terminal.kill`: the terminal finishes; every viewer gets CLOSE `killed`.
    fn kill(&self, address: &str) -> Result<Value, Fail> {
        let (rec, streams) = {
            let mut core = self.core.lock().unwrap();
            let t = core.terminals.iter_mut().find(|t| t["address"] == address).ok_or(("not_found", format!("no terminal \"{address}\"")))?;
            let was_live = t["state"] != "finished";
            if was_live {
                t["state"] = json!("finished");
                t["reason"] = json!("killed");
                t["foreground"] = Value::Null;
                t["status"] = Value::Null;
            }
            let rec = t.clone();
            (rec, if was_live { core.streams.remove(address).unwrap_or_default() } else { Vec::new() })
        };
        let reason = json!({"reason": "killed", "exit_code": null, "signal": null}).to_string().into_bytes();
        for s in streams {
            s.close(reason.clone());
        }
        Ok(rec)
    }

    /// `terminal.rm`: only a finished terminal can go.
    fn rm(&self, address: &str) -> Result<Value, Fail> {
        let mut core = self.core.lock().unwrap();
        let i = core.terminals.iter().position(|t| t["address"] == address).ok_or(("not_found", format!("no terminal \"{address}\"")))?;
        if core.terminals[i]["state"] != "finished" {
            return Err(("live", format!("terminal \"{address}\" is still running")));
        }
        core.terminals.remove(i);
        core.screens.remove(address);
        Ok(json!({"removed": address}))
    }

    /// The viewers of `address`, without those that are gone.
    fn viewers(&self, address: &str) -> Vec<Arc<Stream>> {
        let mut core = self.core.lock().unwrap();
        let list = core.streams.entry(address.to_owned()).or_default();
        list.retain(|s| !s.is_done());
        list.clone()
    }

    fn set_size(&self, address: &str, cols: u64, rows: u64) {
        if let Some(t) = self.core.lock().unwrap().terminals.iter_mut().find(|t| t["address"] == address) {
            t["cols"] = json!(cols);
            t["rows"] = json!(rows);
        }
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

/// An error answer: `(kind, message)`.
type Fail = (&'static str, String);

// ───────────────────────────── stream (core side) ─────────────────────────────

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

fn send_frame(w: &SharedWriter, channel: u32, kind: Kind, payload: &[u8]) -> io::Result<()> {
    let bytes = encode(channel, kind, payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let mut w = w.lock().unwrap();
    w.write_all(&bytes)?;
    w.flush()
}

enum Item {
    Data(Vec<u8>),
    /// A CLOSE frame with this payload (empty = the answer to a viewer's detach).
    Close(Vec<u8>),
}

struct StreamState {
    window: SendWindow,
    queue: VecDeque<Item>,
    /// Held back until the attach reply is written: the reply precedes the stream frames.
    held: bool,
    done: bool,
}

/// One attached viewer: output waits for credit, in order, and CLOSE comes after the output.
struct Stream {
    channel: u32,
    terminal: String,
    out: SharedWriter,
    state: Mutex<StreamState>,
}

impl Stream {
    fn new(channel: u32, terminal: &str, out: SharedWriter) -> Arc<Stream> {
        Arc::new(Stream {
            channel,
            terminal: terminal.to_owned(),
            out,
            state: Mutex::new(StreamState { window: SendWindow::default(), queue: VecDeque::new(), held: true, done: false }),
        })
    }

    fn is_done(&self) -> bool {
        self.state.lock().unwrap().done
    }

    fn push(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut st = self.state.lock().unwrap();
        if !st.done {
            st.queue.push_back(Item::Data(bytes.to_vec()));
        }
        Self::pump(&mut st, &self.out, self.channel);
    }

    fn close(&self, payload: Vec<u8>) {
        let mut st = self.state.lock().unwrap();
        if !st.done {
            st.queue.push_back(Item::Close(payload));
        }
        Self::pump(&mut st, &self.out, self.channel);
    }

    fn grant(&self, n: u32) {
        let mut st = self.state.lock().unwrap();
        st.window.grant(n);
        Self::pump(&mut st, &self.out, self.channel);
    }

    fn release(&self) {
        let mut st = self.state.lock().unwrap();
        st.held = false;
        Self::pump(&mut st, &self.out, self.channel);
    }

    fn pump(st: &mut StreamState, out: &SharedWriter, channel: u32) {
        while !st.held && !st.done {
            let Some(item) = st.queue.pop_front() else { break };
            match item {
                Item::Data(mut d) => {
                    let n = st.window.take(d.len().min(CHUNK));
                    if n == 0 {
                        st.queue.push_front(Item::Data(d));
                        break;
                    }
                    let rest = d.split_off(n);
                    if !rest.is_empty() {
                        st.queue.push_front(Item::Data(rest));
                    }
                    if send_frame(out, channel, Kind::Data, &d).is_err() {
                        st.done = true;
                    }
                }
                Item::Close(p) => {
                    let _ = send_frame(out, channel, Kind::Close, &p);
                    st.done = true;
                }
            }
        }
        if st.done {
            st.queue.clear();
        }
    }
}

// ───────────────────────────── server ─────────────────────────────

/// One connection: the fixture, the line, and the viewers attached through it.
struct Ctx {
    fx: Arc<Fixture>,
    writer: SharedWriter,
    framed: AtomicBool,
    log_path: Option<String>,
    streams: Mutex<HashMap<u32, Arc<Stream>>>,
    next_channel: AtomicU32,
    /// Attached streams to release once the attach reply is written.
    to_release: Mutex<Vec<Arc<Stream>>>,
}

impl Ctx {
    fn is_framed(&self) -> bool {
        self.framed.load(Ordering::Acquire)
    }

    /// One JSON-RPC message to the viewer: a line, or a DATA frame on channel 0 once framed.
    fn send_json(&self, v: &Value) -> io::Result<()> {
        if self.is_framed() {
            return send_frame(&self.writer, 0, Kind::Data, v.to_string().as_bytes());
        }
        let mut line = v.to_string();
        line.push('\n');
        let mut w = self.writer.lock().unwrap();
        w.write_all(line.as_bytes())?;
        w.flush()
    }

    fn log(&self, line: &Value) {
        let Some(path) = &self.log_path else { return };
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }

    fn respond(self: &Arc<Self>, req: &Value) -> Value {
        let (id, method, params) = (req["id"].clone(), req["method"].as_str().unwrap_or("").to_owned(), req["params"].clone());
        if MUTATING.contains(&method.as_str()) {
            self.log(&json!({"method": method, "params": params}));
        }
        if let Some(e) = self.fx.error_for(&method) {
            return json!({"jsonrpc": "2.0", "id": id, "error": e});
        }
        match self.handle(&method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((kind, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message, "data": {"kind": kind, "hint": null}}}),
        }
    }

    fn address(params: &Value) -> &str {
        params["terminal"].as_str().unwrap_or("")
    }

    fn handle(self: &Arc<Self>, method: &str, params: &Value) -> Result<Value, Fail> {
        let fx = &self.fx;
        match method {
            "hello" => Ok(fx.hello(params["framed"].as_bool().unwrap_or(false))),
            "server.status" => Ok(fx.status()),
            "terminal.list" => {
                let want = params["labels"].as_object();
                let terminals: Vec<Value> = fx.terminals().into_iter().filter(|t| want.is_none_or(|w| w.iter().all(|(k, v)| t["labels"].get(k) == Some(v)))).collect();
                Ok(json!({"terminals": terminals, "hidden_finished": 0, "machines": []}))
            }
            "terminal.get" => fx.get(Self::address(params)),
            "terminal.run" => fx.run(params),
            "terminal.kill" => fx.kill(Self::address(params)),
            "terminal.rm" => fx.rm(Self::address(params)),
            "terminal.attach" => self.attach(params),
            "events.subscribe" => {
                let (events, me) = (fx.timed_events(), self.clone());
                let _ = std::thread::Builder::new().name("hc-fake-events".into()).spawn(move || {
                    let mut at = 0;
                    for (t, ev) in events {
                        std::thread::sleep(Duration::from_millis(t - at));
                        at = t;
                        if me.send_json(&ev).is_err() {
                            return;
                        }
                    }
                    for seq in 1.. {
                        std::thread::sleep(HEARTBEAT);
                        if me.send_json(&json!({"jsonrpc": "2.0", "method": "heartbeat", "params": {"seq": seq}})).is_err() {
                            return;
                        }
                    }
                });
                Ok(json!({"seq": 0, "terminals": fx.terminals()}))
            }
            "terminal.send" => Err(("unavailable", "the fake core does not serve terminal.send yet".into())),
            m => Err(("unknown_method", format!("unknown method {m}"))),
        }
    }

    fn attach(&self, params: &Value) -> Result<Value, Fail> {
        if !self.is_framed() {
            return Err(("needs_framed", "terminal.attach needs a framed connection (hello framed:true)".into()));
        }
        let address = Self::address(params);
        let mut rec = self.fx.get(address)?;
        if rec["state"] == "finished" {
            return Err(("finished", format!("terminal \"{address}\" has finished")));
        }
        if let (Some(c), Some(r)) = (params["cols"].as_u64(), params["rows"].as_u64()) {
            self.fx.set_size(address, c, r);
            rec["cols"] = json!(c);
            rec["rows"] = json!(r);
        }
        let channel = self.next_channel.fetch_add(1, Ordering::Relaxed);
        let stream = Stream::new(channel, address, self.writer.clone());
        let screen = self.fx.core.lock().unwrap().screens.get(address).cloned().unwrap_or_default();
        stream.push(format!("{screen}\x1b[?9001l").as_bytes());
        self.fx.core.lock().unwrap().streams.entry(address.to_owned()).or_default().push(stream.clone());
        self.streams.lock().unwrap().insert(channel, stream.clone());
        self.to_release.lock().unwrap().push(stream);
        Ok(json!({"channel": channel, "terminal": rec, "automatic_input": false, "resize_notifications": false}))
    }

    fn release_streams(&self) {
        let list: Vec<_> = self.to_release.lock().unwrap().drain(..).collect();
        for s in list {
            s.release();
        }
    }

    /// A frame from the viewer on a stream channel.
    fn stream_frame(&self, channel: u32, kind: Option<Kind>, payload: &[u8]) {
        let Some(stream) = self.streams.lock().unwrap().get(&channel).cloned() else { return };
        let terminal = stream.terminal.clone();
        match kind {
            Some(Kind::Data) => {
                self.log(&json!({"frame": "data", "terminal": terminal, "bytes": String::from_utf8_lossy(payload)}));
                if self.fx.echo() {
                    for v in self.fx.viewers(&terminal) {
                        v.push(payload);
                    }
                }
            }
            Some(Kind::Resize) => {
                if let Some((cols, rows)) = parse_resize(payload) {
                    self.log(&json!({"frame": "resize", "terminal": terminal, "cols": cols, "rows": rows}));
                    self.fx.set_size(&terminal, cols as u64, rows as u64);
                }
            }
            Some(Kind::Credit) => {
                if let Some(n) = parse_credit(payload) {
                    stream.grant(n);
                }
            }
            Some(Kind::Close) => {
                self.log(&json!({"frame": "close", "terminal": terminal}));
                stream.close(Vec::new());
                self.streams.lock().unwrap().remove(&channel);
            }
            _ => {}
        }
    }

    fn detach_all(&self) {
        let all: Vec<_> = self.streams.lock().unwrap().drain().map(|(_, s)| s).collect();
        for s in all {
            s.state.lock().unwrap().done = true;
        }
    }
}

/// Serve one connection until the client hangs up. Blocks: run it on its own thread.
pub fn serve(fx: Arc<Fixture>, reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) {
    let log_path = std::env::var("CONTR_HC_FAKE_LOG").ok().filter(|p| !p.is_empty());
    serve_logged(fx, reader, writer, log_path);
}

/// `serve` with an explicit log file instead of `CONTR_HC_FAKE_LOG`.
pub fn serve_logged(fx: Arc<Fixture>, reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, log_path: Option<String>) {
    let ctx = Arc::new(Ctx {
        fx,
        writer: Arc::new(Mutex::new(writer)),
        framed: AtomicBool::new(false),
        log_path,
        streams: Mutex::default(),
        next_channel: AtomicU32::new(1),
        to_release: Mutex::default(),
    });
    let mut lines = BufReader::new(reader);
    let mut buf = String::new();
    loop {
        buf.clear();
        match lines.read_line(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Ok(req) = serde_json::from_str::<Value>(buf.trim()) else { continue };
        let reply = ctx.respond(&req);
        if ctx.send_json(&reply).is_err() {
            return;
        }
        if req["method"] == "hello" && req["params"]["framed"] == true && reply.get("error").is_none() {
            ctx.framed.store(true, Ordering::Release);
            serve_frames(&ctx, lines);
            ctx.detach_all();
            return;
        }
    }
}

fn serve_frames(ctx: &Arc<Ctx>, mut reader: BufReader<Box<dyn Read + Send>>) {
    let mut dec = Decoder::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let Ok(frames) = dec.push(&chunk[..n]) else { return };
        for f in frames {
            if f.channel != 0 {
                ctx.stream_frame(f.channel, f.kind(), &f.payload);
            } else if f.kind() == Some(Kind::Data) {
                let Ok(req) = serde_json::from_slice::<Value>(&f.payload) else { continue };
                let reply = ctx.respond(&req);
                if ctx.send_json(&reply).is_err() {
                    return;
                }
                ctx.release_streams();
            }
        }
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
        assert_eq!(c.call("terminal.kill", json!({"terminal": "t9"}), T).unwrap_err().kind(), Some("not_found"));
        let c = connect(json!({}), None);
        assert_eq!(c.call("terminal.send", json!({}), T).unwrap_err().kind(), Some("unavailable"));
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

    // ───── framed mode ─────

    use crate::hc::framed::{FramedConn, START_CREDIT, StreamEvent};

    fn framed(fx: Value, log: Option<String>) -> (FramedConn, Arc<Fixture>) {
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        let fx = Arc::new(Fixture::from_value(fx));
        let f2 = fx.clone();
        std::thread::spawn(move || serve_logged(f2, Box::new(req_r), Box::new(rep_w), log));
        (FramedConn::connect(Box::new(rep_r), Box::new(req_w), None, T).unwrap().0, fx)
    }

    fn attach(c: &FramedConn, t: &str) -> (u32, Receiver<StreamEvent>) {
        let r = c.call("terminal.attach", json!({"terminal": t, "cols": 100, "rows": 30, "history": 1000}), T).unwrap();
        let ch = r["channel"].as_u64().unwrap() as u32;
        (ch, c.open_stream(ch))
    }

    /// Read at least `want` bytes of stream output.
    fn data(rx: &Receiver<StreamEvent>, want: usize) -> Vec<u8> {
        let mut got = Vec::new();
        while got.len() < want {
            match rx.recv_timeout(T).unwrap() {
                StreamEvent::Data(d) => got.extend(d),
                other => panic!("{other:?}"),
            }
        }
        got
    }

    /// The first paint of a screen with no text: only the win32-input-mode reset.
    const PAINT: &[u8] = b"\x1b[?9001l";

    fn log_file(name: &str) -> (std::path::PathBuf, Option<String>) {
        let p = std::env::temp_dir().join(format!("contr-rune-fake-{name}-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let s = Some(p.to_str().unwrap().to_owned());
        (p, s)
    }

    fn log_lines(p: &std::path::Path) -> Vec<Value> {
        let text = std::fs::read_to_string(p).unwrap();
        let _ = std::fs::remove_file(p);
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn attach_needs_a_framed_connection() {
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        let fx = Arc::new(Fixture::from_value(json!({"terminals": [{"address": "t1", "state": "live"}]})));
        std::thread::spawn(move || serve(fx, Box::new(req_r), Box::new(rep_w)));
        let plain = Conn::new(Box::new(rep_r), Box::new(req_w), None);
        assert_eq!(plain.call("hello", json!({"generation": 1}), T).unwrap()["framed"], false);
        assert_eq!(plain.call("terminal.attach", json!({"terminal": "t1"}), T).unwrap_err().kind(), Some("needs_framed"));
    }

    #[test]
    fn run_attach_first_paint_input_resize_credit_and_close_with_a_reason() {
        let (log, path) = log_file("flow");
        let (c, _) = framed(json!({"echo": true}), path);
        let run = c.call("terminal.run", json!({"argv": [], "cwd": "/tmp", "name": "term-1", "labels": {"contr": "agent", "role": "terminal", "project": "p"}, "size": {"cols": 100, "rows": 30}}), T).unwrap();
        assert_eq!((run["address"].as_str(), run["labels"]["role"].as_str(), run["cols"].as_u64()), (Some("t1"), Some("terminal"), Some(100)));
        let (ch, rx) = attach(&c, "t1");
        assert_eq!(data(&rx, 10), b"$ \x1b[?9001l");
        // DATA is echoed back as output.
        c.data(ch, b"ls\r").unwrap();
        assert_eq!(data(&rx, 3), b"ls\r");
        c.resize(ch, 120, 40).unwrap();
        c.credit(ch, 11).unwrap();
        c.focus(ch, true).unwrap();
        // Kill closes the stream with a reason, on the same connection (this also orders the
        // frames above before the call, as they share one line).
        c.call("terminal.kill", json!({"terminal": "t1"}), T).unwrap();
        match rx.recv_timeout(T).unwrap() {
            StreamEvent::Close(i) => assert_eq!((i.reason.as_deref(), i.exit_code), (Some("killed"), None)),
            other => panic!("{other:?}"),
        }
        let lines = log_lines(&log);
        assert_eq!(lines[0]["method"], "terminal.run");
        assert_eq!(lines[1], json!({"frame": "data", "terminal": "t1", "bytes": "ls\r"}));
        assert_eq!(lines[2], json!({"frame": "resize", "terminal": "t1", "cols": 120, "rows": 40}));
        assert_eq!(lines[3], json!({"method": "terminal.kill", "params": {"terminal": "t1"}}));
        assert_eq!(c.call("terminal.get", json!({"terminal": "t1"}), T).unwrap()["cols"], 120);
    }

    #[test]
    fn a_viewer_close_is_answered_with_an_empty_close_and_logged() {
        let (log, path) = log_file("close");
        let (c, _) = framed(json!({"terminals": [{"address": "t3", "state": "live"}], "screens": {"t3": "hi\r\n"}}), path);
        let (ch, rx) = attach(&c, "t3");
        assert_eq!(data(&rx, 12), b"hi\r\n\x1b[?9001l");
        c.close(ch).unwrap();
        match rx.recv_timeout(T).unwrap() {
            StreamEvent::Close(i) => assert!(i.is_ack()),
            other => panic!("{other:?}"),
        }
        assert_eq!(log_lines(&log)[0], json!({"frame": "close", "terminal": "t3"}));
    }

    #[test]
    fn output_waits_for_credit_and_keeps_its_order() {
        let (c, fx) = framed(json!({"terminals": [{"address": "t1", "state": "live"}]}), None);
        let (ch, rx) = attach(&c, "t1");
        assert_eq!(data(&rx, PAINT.len()), PAINT);
        // 300 KiB of patterned bytes: only the 256 KiB credit (less the first paint) goes out.
        let big: Vec<u8> = (0..300 * 1024).map(|i| (i % 251) as u8).collect();
        for v in fx.viewers("t1") {
            v.push(&big);
        }
        let room = START_CREDIT as usize - PAINT.len();
        assert_eq!(data(&rx, room), big[..room]);
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "sent past the credit");
        c.credit(ch, 1024 * 1024).unwrap();
        assert_eq!(data(&rx, big.len() - room), big[room..]);
    }

    #[test]
    fn close_waits_behind_output_that_has_no_credit() {
        let (c, fx) = framed(json!({"terminals": [{"address": "t1", "state": "live"}]}), None);
        let (ch, rx) = attach(&c, "t1");
        data(&rx, PAINT.len());
        for v in fx.viewers("t1") {
            v.push(&vec![b'x'; START_CREDIT as usize + 100]);
        }
        data(&rx, START_CREDIT as usize - PAINT.len());
        c.call("terminal.kill", json!({"terminal": "t1"}), T).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "CLOSE overtook the data");
        c.credit(ch, 1000).unwrap();
        assert_eq!(data(&rx, 108), vec![b'x'; 108]);
        assert!(matches!(rx.recv_timeout(T).unwrap(), StreamEvent::Close(_)));
    }

    #[test]
    fn echo_reaches_every_viewer_of_the_terminal() {
        let fx = json!({"echo": true, "terminals": [{"address": "t1", "state": "live"}]});
        let (a, fxa) = framed(fx, None);
        // A second connection to the same fixture core.
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        std::thread::spawn(move || serve_logged(fxa, Box::new(req_r), Box::new(rep_w), None));
        let b = FramedConn::connect(Box::new(rep_r), Box::new(req_w), None, T).unwrap().0;
        let (cha, rxa) = attach(&a, "t1");
        let (_, rxb) = attach(&b, "t1");
        data(&rxa, PAINT.len());
        data(&rxb, PAINT.len());
        a.data(cha, b"hey").unwrap();
        assert_eq!(data(&rxa, 3), b"hey");
        assert_eq!(data(&rxb, 3), b"hey");
    }

    #[test]
    fn without_echo_input_is_not_written_back() {
        let (c, _) = framed(json!({"terminals": [{"address": "t1", "state": "live"}]}), None);
        let (ch, rx) = attach(&c, "t1");
        data(&rx, PAINT.len());
        c.data(ch, b"nope").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
    }

    #[test]
    fn attach_errors_and_the_run_kill_rm_rules() {
        let (c, _) = framed(json!({"terminals": [{"address": "t7", "name": "a", "state": "live"}]}), None);
        assert_eq!(c.call("terminal.attach", json!({"terminal": "t9"}), T).unwrap_err().kind(), Some("not_found"));
        let run = |name: &str| c.call("terminal.run", json!({"name": name, "cwd": "/tmp"}), T);
        assert_eq!(run("a").unwrap_err().kind(), Some("name_taken"));
        assert_eq!(run("b").unwrap()["address"], "t8");
        assert_eq!(run("c").unwrap()["address"], "t9");
        assert_eq!(c.call("terminal.rm", json!({"terminal": "t8"}), T).unwrap_err().kind(), Some("live"));
        assert_eq!(c.call("terminal.kill", json!({"terminal": "t8"}), T).unwrap()["state"], "finished");
        assert_eq!(c.call("terminal.attach", json!({"terminal": "t8"}), T).unwrap_err().kind(), Some("finished"));
        assert_eq!(c.call("terminal.rm", json!({"terminal": "t8"}), T).unwrap(), json!({"removed": "t8"}));
        assert_eq!(c.call("terminal.rm", json!({"terminal": "t8"}), T).unwrap_err().kind(), Some("not_found"));
        // A finished name can be used again; the list shows what is left.
        assert_eq!(run("b").unwrap()["address"], "t10");
        let list = c.call("terminal.list", json!({}), T).unwrap();
        assert_eq!(list["terminals"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn events_arrive_as_channel_zero_frames_in_framed_mode() {
        let (ntx, nrx) = channel();
        let (req_r, req_w) = pipe();
        let (rep_r, rep_w) = pipe();
        let fx = Arc::new(Fixture::from_value(json!({"events": [{"after_ms": 10, "method": "event", "params": {"seq": 1, "type": "title", "terminal": "t1", "data": {"title": "x"}}}]})));
        std::thread::spawn(move || serve(fx, Box::new(req_r), Box::new(rep_w)));
        let c = FramedConn::connect(Box::new(rep_r), Box::new(req_w), Some(ntx), T).unwrap().0;
        c.call("events.subscribe", json!({}), T).unwrap();
        let n = nrx.recv_timeout(T).unwrap();
        assert_eq!((n.method.as_str(), n.params["type"].as_str()), ("event", Some("title")));
    }
}
