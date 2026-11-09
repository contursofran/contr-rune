//! The framed hc connection (Contract C2; `docs/design/hc/API.md` "Framed mode").
//!
//! Frame: `[channel u32 LE][kind u8][len u32 LE][payload]`, payload at most 16 MiB. Channel 0
//! carries JSON-RPC both ways (one message per DATA frame); every other channel is a terminal
//! stream. `FramedConn` routes channel 0 like `Conn` does and streams to per-channel receivers.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::conn::{HcError, Note};
use super::wire::{HelloResult, RpcError};

/// Largest payload of one frame.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
/// Credit a stream starts with (bytes).
pub const START_CREDIT: u32 = 256 * 1024;

/// Frame kinds ([hcA] table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Data = 0,
    Credit = 1,
    Close = 2,
    Resize = 3,
    Focus = 4,
    Facts = 5,
    AutomaticInput = 6,
}

impl Kind {
    pub fn from_u8(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::Data,
            1 => Kind::Credit,
            2 => Kind::Close,
            3 => Kind::Resize,
            4 => Kind::Focus,
            5 => Kind::Facts,
            6 => Kind::AutomaticInput,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The payload is over 16 MiB (the size is the declared or given length).
    TooLarge(usize),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooLarge(n) => write!(f, "frame payload of {n} bytes is over the 16 MiB limit"),
        }
    }
}

/// One decoded frame. `kind` stays a raw byte: a newer core may send kinds we do not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn kind(&self) -> Option<Kind> {
        Kind::from_u8(self.kind)
    }
}

/// Encode one frame.
pub fn encode(channel: u32, kind: Kind, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(&channel.to_le_bytes());
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Incremental decoder: feed it whatever the line gives, in any split.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    failed: Option<usize>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Add bytes and return every frame that is now complete. A length over 16 MiB is an error
    /// and stays one (the stream cannot be re-synchronised): drop the connection.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, FrameError> {
        if let Some(n) = self.failed {
            return Err(FrameError::TooLarge(n));
        }
        self.buf.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut at = 0;
        while self.buf.len() - at >= 9 {
            let h = &self.buf[at..at + 9];
            let channel = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
            let kind = h[4];
            let len = u32::from_le_bytes([h[5], h[6], h[7], h[8]]) as usize;
            if len > MAX_PAYLOAD {
                self.failed = Some(len);
                self.buf.clear();
                return Err(FrameError::TooLarge(len));
            }
            if self.buf.len() - at - 9 < len {
                break;
            }
            frames.push(Frame { channel, kind, payload: self.buf[at + 9..at + 9 + len].to_vec() });
            at += 9 + len;
        }
        self.buf.drain(..at);
        Ok(frames)
    }

    /// Bytes held for an incomplete frame.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

// ───────────────────────────── payloads ─────────────────────────────

pub fn credit_payload(n: u32) -> [u8; 4] {
    n.to_le_bytes()
}

pub fn parse_credit(p: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(p.try_into().ok()?))
}

pub fn resize_payload(cols: u16, rows: u16) -> [u8; 4] {
    let (c, r) = (cols.to_le_bytes(), rows.to_le_bytes());
    [c[0], c[1], r[0], r[1]]
}

pub fn parse_resize(p: &[u8]) -> Option<(u16, u16)> {
    let p: [u8; 4] = p.try_into().ok()?;
    Some((u16::from_le_bytes([p[0], p[1]]), u16::from_le_bytes([p[2], p[3]])))
}

pub fn focus_payload(gained: bool) -> [u8; 1] {
    [gained as u8]
}

/// Why a stream closed. An empty CLOSE is the core's answer to our own detach (`reason: None`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CloseInfo {
    pub reason: Option<String>,
    pub exit_code: Option<i64>,
    pub signal: Option<Value>,
}

impl CloseInfo {
    pub fn parse(payload: &[u8]) -> CloseInfo {
        if payload.iter().all(u8::is_ascii_whitespace) {
            return CloseInfo::default();
        }
        let v: Value = serde_json::from_slice(payload).unwrap_or(Value::Null);
        CloseInfo {
            reason: v["reason"].as_str().map(str::to_owned),
            exit_code: v["exit_code"].as_i64(),
            signal: v.get("signal").filter(|s| !s.is_null()).cloned(),
        }
    }

    /// The acknowledgement of our own CLOSE.
    #[cfg(test)]
    pub fn is_ack(&self) -> bool {
        self.reason.is_none()
    }
}

// ───────────────────────────── credit accounting ─────────────────────────────

/// The sending side of a stream (the core, and the fake): bytes it may still send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendWindow {
    avail: u64,
}

impl Default for SendWindow {
    fn default() -> Self {
        SendWindow { avail: START_CREDIT as u64 }
    }
}

impl SendWindow {
    #[cfg(test)]
    pub fn available(&self) -> u64 {
        self.avail
    }

    /// A CREDIT frame arrived.
    pub fn grant(&mut self, n: u32) {
        self.avail += n as u64;
    }

    /// Take up to `want` bytes of credit; returns how many may be sent now.
    pub fn take(&mut self, want: usize) -> usize {
        let n = (want as u64).min(self.avail) as usize;
        self.avail -= n as u64;
        n
    }
}

/// The receiving side (the viewer): counts bytes written out and says when to send CREDIT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    unacked: u64,
}

impl Ledger {
    /// `n` bytes were consumed (written to the terminal).
    pub fn consumed(&mut self, n: usize) {
        self.unacked += n as u64;
    }

    /// The CREDIT to send now, if any (call after each batch). The counter restarts at zero; an
    /// amount over `u32::MAX` is sent in several frames by calling again.
    pub fn take(&mut self) -> Option<u32> {
        if self.unacked == 0 {
            return None;
        }
        let n = self.unacked.min(u32::MAX as u64) as u32;
        self.unacked -= n as u64;
        Some(n)
    }
}

// ───────────────────────────── connection ─────────────────────────────

/// What a stream channel delivers.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    /// Output bytes (first paint, then live output).
    Data(Vec<u8>),
    /// The core announces a grid change (observers only).
    Resize { cols: u16, rows: u16 },
    /// The channel is over.
    Close(CloseInfo),
    /// The line was lost: no more events will come.
    Lost,
}

/// Events for a channel nobody opened yet are kept (the attach reply and the first paint can
/// arrive back to back); this caps how many.
const EARLY_CAP: usize = 4096;

#[derive(Default)]
struct Streams {
    sinks: HashMap<u32, Sender<StreamEvent>>,
    early: HashMap<u32, Vec<StreamEvent>>,
}

type Pending = Mutex<HashMap<u64, Sender<Result<Value, HcError>>>>;

struct Shared {
    pending: Pending,
    streams: Mutex<Streams>,
    closed: AtomicBool,
}

pub struct FramedConn {
    writer: Mutex<Box<dyn Write + Send>>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
}

impl FramedConn {
    /// Send the plain `hello {"generation":1,"framed":true}`, wait for its reply (refusing any
    /// other generation and a core that does not frame) and switch to frames.
    pub fn connect(reader: Box<dyn Read + Send>, mut writer: Box<dyn Write + Send>, notes: Option<Sender<Note>>, timeout: Duration) -> Result<(FramedConn, HelloResult), HcError> {
        let shared = Arc::new(Shared { pending: Mutex::default(), streams: Mutex::default(), closed: AtomicBool::new(false) });
        let (hello_tx, hello_rx) = channel::<Result<HelloResult, HcError>>();
        let s = shared.clone();
        let spawned = std::thread::Builder::new().name("hc-frames".into()).spawn(move || {
            let mut reader = BufReader::new(reader);
            let hello = read_hello(&mut reader);
            let ok = hello.is_ok();
            let _ = hello_tx.send(hello);
            if ok {
                frame_loop(reader, &s, notes);
            }
            s.closed.store(true, Ordering::Release);
            s.pending.lock().unwrap().clear();
            let mut st = s.streams.lock().unwrap();
            for (_, tx) in st.sinks.drain() {
                let _ = tx.send(StreamEvent::Lost);
            }
            st.early.clear();
        });
        spawned.map_err(HcError::Io)?;
        let req = json!({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"generation": 1, "framed": true}});
        let line = format!("{req}\n");
        writer.write_all(line.as_bytes()).and_then(|()| writer.flush()).map_err(|e| if e.kind() == io::ErrorKind::BrokenPipe { HcError::NotSent } else { HcError::Io(e) })?;
        let hello = match hello_rx.recv_timeout(timeout) {
            Ok(r) => r?,
            Err(RecvTimeoutError::Timeout) => return Err(HcError::Io(io::Error::new(io::ErrorKind::TimedOut, "hello: no reply"))),
            Err(RecvTimeoutError::Disconnected) => return Err(HcError::Closed),
        };
        if hello.generation != 1 {
            return Err(HcError::Rpc { kind: "generation".into(), message: format!("unsupported hc protocol generation {} (need 1)", hello.generation) });
        }
        if !hello.framed {
            return Err(HcError::Rpc { kind: "needs_framed".into(), message: "the core did not switch to framed mode".into() });
        }
        Ok((FramedConn { writer: Mutex::new(writer), shared, next_id: AtomicU64::new(1) }, hello))
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// A JSON-RPC call on channel 0.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HcError> {
        if self.is_closed() {
            return Err(HcError::NotSent);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.shared.pending.lock().unwrap().insert(id, tx);
        if self.is_closed() {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(HcError::NotSent);
        }
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        if let Err(e) = self.send(0, Kind::Data, msg.as_bytes()) {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                self.shared.pending.lock().unwrap().remove(&id);
                Err(HcError::Io(io::Error::new(io::ErrorKind::TimedOut, format!("{method}: no reply"))))
            }
            Err(RecvTimeoutError::Disconnected) => Err(HcError::Closed),
        }
    }

    /// Start receiving a stream channel (the `channel` of an attach reply). Frames that arrived
    /// before this call are delivered first.
    pub fn open_stream(&self, channel_id: u32) -> Receiver<StreamEvent> {
        let (tx, rx) = channel();
        let mut st = self.shared.streams.lock().unwrap();
        if self.is_closed() {
            let _ = tx.send(StreamEvent::Lost);
            return rx;
        }
        for ev in st.early.remove(&channel_id).unwrap_or_default() {
            let _ = tx.send(ev);
        }
        st.sinks.insert(channel_id, tx);
        rx
    }

    pub fn send(&self, channel_id: u32, kind: Kind, payload: &[u8]) -> Result<(), HcError> {
        if self.is_closed() {
            return Err(HcError::NotSent);
        }
        let bytes = encode(channel_id, kind, payload).map_err(|e| HcError::Io(io::Error::new(io::ErrorKind::InvalidInput, e.to_string())))?;
        let mut w = self.writer.lock().unwrap();
        w.write_all(&bytes).and_then(|()| w.flush()).map_err(|e| if e.kind() == io::ErrorKind::BrokenPipe { HcError::NotSent } else { HcError::Io(e) })
    }

    /// Key bytes to the program.
    pub fn data(&self, channel_id: u32, bytes: &[u8]) -> Result<(), HcError> {
        self.send(channel_id, Kind::Data, bytes)
    }

    pub fn credit(&self, channel_id: u32, n: u32) -> Result<(), HcError> {
        self.send(channel_id, Kind::Credit, &credit_payload(n))
    }

    pub fn resize(&self, channel_id: u32, cols: u16, rows: u16) -> Result<(), HcError> {
        self.send(channel_id, Kind::Resize, &resize_payload(cols, rows))
    }

    pub fn focus(&self, channel_id: u32, gained: bool) -> Result<(), HcError> {
        self.send(channel_id, Kind::Focus, &focus_payload(gained))
    }

    /// Detach: the core answers with an empty CLOSE.
    pub fn close(&self, channel_id: u32) -> Result<(), HcError> {
        self.send(channel_id, Kind::Close, &[])
    }
}

/// The plain hello reply: one line, read before any frame (and without reading past it).
fn read_hello(reader: &mut BufReader<Box<dyn Read + Send>>) -> Result<HelloResult, HcError> {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Err(HcError::Closed),
            Err(e) => return Err(HcError::Io(e)),
            Ok(_) => {}
        }
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if v.get("id").is_none() {
            continue;
        }
        if let Some(e) = v.get("error") {
            return Err(HcError::from_rpc(serde_json::from_value::<RpcError>(e.clone()).unwrap_or_default()));
        }
        return serde_json::from_value(v.get("result").cloned().unwrap_or(Value::Null)).map_err(|e| HcError::Io(io::Error::new(io::ErrorKind::InvalidData, format!("bad hello reply: {e}"))));
    }
}

fn frame_loop(mut reader: BufReader<Box<dyn Read + Send>>, shared: &Shared, notes: Option<Sender<Note>>) {
    let mut dec = Decoder::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        // `BufReader` hands out what the hello read already buffered before it reads more.
        let n = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let Ok(frames) = dec.push(&chunk[..n]) else { return };
        for f in frames {
            route(f, shared, &notes);
        }
    }
}

fn route(f: Frame, shared: &Shared, notes: &Option<Sender<Note>>) {
    if f.channel == 0 {
        if f.kind() == Some(Kind::Data) {
            route_rpc(&f.payload, shared, notes);
        }
        return;
    }
    let ev = match f.kind() {
        Some(Kind::Data) => StreamEvent::Data(f.payload),
        Some(Kind::Resize) => match parse_resize(&f.payload) {
            Some((cols, rows)) => StreamEvent::Resize { cols, rows },
            None => return,
        },
        Some(Kind::Close) => StreamEvent::Close(CloseInfo::parse(&f.payload)),
        _ => return,
    };
    let closes = matches!(ev, StreamEvent::Close(_));
    let mut st = shared.streams.lock().unwrap();
    match st.sinks.get(&f.channel) {
        Some(tx) => {
            if tx.send(ev).is_err() || closes {
                st.sinks.remove(&f.channel);
            }
        }
        None => {
            let early = st.early.entry(f.channel).or_default();
            if early.len() < EARLY_CAP {
                early.push(ev);
            }
        }
    }
}

fn route_rpc(payload: &[u8], shared: &Shared, notes: &Option<Sender<Note>>) {
    let Ok(v) = serde_json::from_slice::<Value>(payload) else { return };
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        let reply = match v.get("error") {
            Some(e) => Err(HcError::from_rpc(serde_json::from_value(e.clone()).unwrap_or_default())),
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        };
        if let Some(tx) = shared.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(reply);
        }
    } else if let (Some(method), Some(n)) = (v.get("method").and_then(Value::as_str), notes) {
        let _ = n.send(Note { method: method.to_owned(), params: v.get("params").cloned().unwrap_or(Value::Null) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::fake::{PipeReader, PipeWriter, pipe};

    const T: Duration = Duration::from_secs(5);

    fn hello_line(framed: bool, generation: u32) -> String {
        format!("{}\n", json!({"jsonrpc": "2.0", "id": 0, "result": {"generation": generation, "version": "0.1.19", "framed": framed, "core": {"id": "x", "name": "y"}}}))
    }

    /// A conn wired to a scripted core: the core's reader (requests) and writer (replies).
    fn rig(first: &str, notes: Option<Sender<Note>>) -> Result<(FramedConn, PipeReader, PipeWriter), HcError> {
        let (req_r, req_w) = pipe();
        let (rep_r, mut rep_w) = pipe();
        rep_w.write_all(first.as_bytes()).unwrap();
        let (c, _) = FramedConn::connect(Box::new(rep_r), Box::new(req_w), notes, T)?;
        // Take the plain hello request off the line: what follows is frames only.
        let (mut req_r, mut byte, mut line) = (req_r, [0u8; 1], Vec::new());
        while byte[0] != b'\n' {
            req_r.read_exact(&mut byte).unwrap();
            line.push(byte[0]);
        }
        let hello: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!((hello["method"].as_str(), hello["params"].clone()), (Some("hello"), json!({"generation": 1, "framed": true})));
        Ok((c, req_r, rep_w))
    }

    fn read_frames(core: &mut PipeReader, want: usize) -> Vec<Frame> {
        let mut dec = Decoder::new();
        let mut out = Vec::new();
        let mut b = [0u8; 4096];
        while out.len() < want {
            let n = core.read(&mut b).unwrap();
            assert!(n > 0, "core line ended");
            out.extend(dec.push(&b[..n]).unwrap());
        }
        out
    }

    #[test]
    fn a_frame_round_trips() {
        let bytes = encode(7, Kind::Data, b"hi").unwrap();
        assert_eq!(bytes, [7, 0, 0, 0, 0, 2, 0, 0, 0, b'h', b'i']);
        let frames = Decoder::new().push(&bytes).unwrap();
        assert_eq!(frames, vec![Frame { channel: 7, kind: 0, payload: b"hi".to_vec() }]);
        assert_eq!(frames[0].kind(), Some(Kind::Data));
        assert_eq!(Kind::from_u8(9), None);
    }

    #[test]
    fn partial_reads_and_several_frames_per_read() {
        let mut all = encode(1, Kind::Data, b"abc").unwrap();
        all.extend(encode(2, Kind::Close, &[]).unwrap());
        all.extend(encode(3, Kind::Resize, &resize_payload(80, 24)).unwrap());
        let mut dec = Decoder::new();
        let mut got = Vec::new();
        for b in &all {
            got.extend(dec.push(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(got.len(), 3);
        assert_eq!(dec.pending(), 0);
        assert_eq!(got[0].payload, b"abc");
        assert_eq!((got[1].channel, got[1].kind(), got[1].payload.len()), (2, Some(Kind::Close), 0));
        assert_eq!(parse_resize(&got[2].payload), Some((80, 24)));
        assert_eq!(Decoder::new().push(&all).unwrap().len(), 3);
        // A header alone yields nothing and is kept.
        let mut dec = Decoder::new();
        assert!(dec.push(&all[..5]).unwrap().is_empty());
        assert_eq!(dec.pending(), 5);
    }

    #[test]
    fn the_16_mib_limit_holds_both_ways() {
        let at_limit = vec![0u8; MAX_PAYLOAD];
        assert_eq!(encode(1, Kind::Data, &at_limit).unwrap().len(), 9 + MAX_PAYLOAD);
        assert_eq!(encode(1, Kind::Data, &vec![0u8; MAX_PAYLOAD + 1]), Err(FrameError::TooLarge(MAX_PAYLOAD + 1)));
        // A declared length over the limit fails at the header, before any payload, and stays failed.
        let mut hdr = vec![1, 0, 0, 0, 0];
        hdr.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
        let mut dec = Decoder::new();
        assert_eq!(dec.push(&hdr), Err(FrameError::TooLarge(MAX_PAYLOAD + 1)));
        assert!(dec.push(&[0]).is_err());
        // A declared length at the limit waits for its payload.
        let mut hdr = vec![1, 0, 0, 0, 0];
        hdr.extend_from_slice(&(MAX_PAYLOAD as u32).to_le_bytes());
        assert!(Decoder::new().push(&hdr).unwrap().is_empty());
    }

    #[test]
    fn small_payloads_encode_and_parse() {
        assert_eq!(parse_credit(&credit_payload(262_144)), Some(262_144));
        assert_eq!(parse_credit(&[1, 2]), None);
        assert_eq!(resize_payload(0x0102, 0x0304), [2, 1, 4, 3]);
        assert_eq!(parse_resize(&[1]), None);
        assert_eq!(focus_payload(true), [1]);
        assert_eq!(focus_payload(false), [0]);
    }

    #[test]
    fn close_payloads_carry_a_reason_or_are_the_ack() {
        let c = CloseInfo::parse(br#"{"reason":"exited","exit_code":0,"signal":null}"#);
        assert_eq!((c.reason.as_deref(), c.exit_code, c.signal.clone()), (Some("exited"), Some(0), None));
        assert!(!c.is_ack());
        let c = CloseInfo::parse(br#"{"reason":"killed","exit_code":null,"signal":"SIGKILL"}"#);
        assert_eq!((c.reason.as_deref(), c.exit_code, c.signal), (Some("killed"), None, Some(json!("SIGKILL"))));
        assert!(CloseInfo::parse(b"").is_ack());
        assert!(CloseInfo::parse(b"not json").is_ack());
    }

    #[test]
    fn credit_starts_at_256_kib_and_is_spent_and_granted() {
        let mut w = SendWindow::default();
        assert_eq!(w.available(), 256 * 1024);
        assert_eq!(w.take(100_000), 100_000);
        assert_eq!(w.take(200_000), 162_144);
        assert_eq!(w.take(1), 0);
        w.grant(500);
        assert_eq!(w.take(1000), 500);
        w.grant(u32::MAX);
        w.grant(u32::MAX);
        assert_eq!(w.available(), 2 * u32::MAX as u64);
    }

    #[test]
    fn the_viewer_ledger_returns_what_it_consumed_once() {
        let mut l = Ledger::default();
        assert_eq!(l.take(), None);
        l.consumed(1000);
        l.consumed(24);
        assert_eq!(l.take(), Some(1024));
        assert_eq!(l.take(), None);
        l.consumed(u32::MAX as usize);
        l.consumed(10);
        assert_eq!(l.take(), Some(u32::MAX));
        assert_eq!(l.take(), Some(10));
        assert_eq!(l.take(), None);
    }

    #[test]
    fn connect_refuses_a_wrong_generation_and_a_core_that_does_not_frame() {
        assert_eq!(rig(&hello_line(true, 2), None).err().unwrap().kind(), Some("generation"));
        assert_eq!(rig(&hello_line(false, 1), None).err().unwrap().kind(), Some("needs_framed"));
        let err = json!({"jsonrpc": "2.0", "id": 0, "error": {"code": -32000, "message": "no", "data": {"kind": "unavailable", "hint": null}}});
        assert_eq!(rig(&format!("{err}\n"), None).err().unwrap().kind(), Some("unavailable"));
    }

    #[test]
    fn channel_zero_routes_replies_and_notes_and_streams_go_to_their_channel() {
        let (ntx, nrx) = channel();
        // The first frames arrive in the same write as the hello line.
        let mut first = hello_line(true, 1).into_bytes();
        first.extend(encode(5, Kind::Data, b"early").unwrap());
        let (conn, mut core, mut out) = rig(std::str::from_utf8(&first).unwrap(), Some(ntx)).unwrap();
        let conn = Arc::new(conn);
        // Data for channel 5 arrived before it was opened: delivered on open, in order.
        out.write_all(&encode(5, Kind::Data, b"-more").unwrap()).unwrap();
        out.write_all(&encode(6, Kind::Data, b"six").unwrap()).unwrap();
        let (s5, s6) = (conn.open_stream(5), conn.open_stream(6));
        assert_eq!(s5.recv_timeout(T).unwrap(), StreamEvent::Data(b"early".to_vec()));
        assert_eq!(s5.recv_timeout(T).unwrap(), StreamEvent::Data(b"-more".to_vec()));
        assert_eq!(s6.recv_timeout(T).unwrap(), StreamEvent::Data(b"six".to_vec()));
        assert!(s5.try_recv().is_err());
        // A call goes out as a DATA frame on channel 0 and its reply comes back by id.
        let c = conn.clone();
        let h = std::thread::spawn(move || c.call("terminal.list", json!({}), T));
        let req = &read_frames(&mut core, 1)[0];
        assert_eq!((req.channel, req.kind()), (0, Some(Kind::Data)));
        let v: Value = serde_json::from_slice(&req.payload).unwrap();
        assert_eq!(v["method"], "terminal.list");
        let note = json!({"jsonrpc": "2.0", "method": "heartbeat", "params": {"seq": 3}});
        out.write_all(&encode(0, Kind::Data, note.to_string().as_bytes()).unwrap()).unwrap();
        let reply = json!({"jsonrpc": "2.0", "id": v["id"], "result": {"terminals": []}});
        out.write_all(&encode(0, Kind::Data, reply.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(h.join().unwrap().unwrap(), json!({"terminals": []}));
        assert_eq!(nrx.recv_timeout(T).unwrap(), Note { method: "heartbeat".into(), params: json!({"seq": 3}) });
        // An error reply keeps its kind.
        let c = conn.clone();
        let h = std::thread::spawn(move || c.call("terminal.get", json!({}), T));
        let req = &read_frames(&mut core, 1)[0];
        let v: Value = serde_json::from_slice(&req.payload).unwrap();
        let err = json!({"jsonrpc": "2.0", "id": v["id"], "error": {"code": -32000, "message": "no t9", "data": {"kind": "not_found", "hint": null}}});
        out.write_all(&encode(0, Kind::Data, err.to_string().as_bytes()).unwrap()).unwrap();
        assert!(matches!(h.join().unwrap(), Err(HcError::NotFound(_))));
    }

    #[test]
    fn stream_frames_decode_and_close_ends_the_stream() {
        let (conn, _core, mut out) = rig(&hello_line(true, 1), None).unwrap();
        let s = conn.open_stream(2);
        out.write_all(&encode(2, Kind::Resize, &resize_payload(90, 30)).unwrap()).unwrap();
        out.write_all(&encode(2, Kind::Facts, b"{}").unwrap()).unwrap(); // not for the viewer: ignored
        out.write_all(&encode(2, Kind::Close, br#"{"reason":"exited","exit_code":0,"signal":null}"#).unwrap()).unwrap();
        assert_eq!(s.recv_timeout(T).unwrap(), StreamEvent::Resize { cols: 90, rows: 30 });
        match s.recv_timeout(T).unwrap() {
            StreamEvent::Close(c) => assert_eq!((c.reason.as_deref(), c.exit_code), (Some("exited"), Some(0))),
            other => panic!("{other:?}"),
        }
        // The sink is dropped after a CLOSE: the receiver sees the end.
        assert!(matches!(s.recv_timeout(T), Err(RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn viewer_frames_are_written_as_the_plan_says() {
        let (conn, mut core, _out) = rig(&hello_line(true, 1), None).unwrap();
        conn.data(4, b"ls\r").unwrap();
        conn.credit(4, 1234).unwrap();
        conn.resize(4, 100, 40).unwrap();
        conn.focus(4, true).unwrap();
        conn.focus(4, false).unwrap();
        conn.close(4).unwrap();
        let f = read_frames(&mut core, 6);
        let kinds: Vec<_> = f.iter().map(|f| (f.channel, f.kind().unwrap())).collect();
        assert_eq!(kinds, vec![(4, Kind::Data), (4, Kind::Credit), (4, Kind::Resize), (4, Kind::Focus), (4, Kind::Focus), (4, Kind::Close)]);
        assert_eq!(f[0].payload, b"ls\r");
        assert_eq!(parse_credit(&f[1].payload), Some(1234));
        assert_eq!(parse_resize(&f[2].payload), Some((100, 40)));
        assert_eq!((f[3].payload.as_slice(), f[4].payload.as_slice(), f[5].payload.len()), (&[1u8][..], &[0u8][..], 0));
    }

    #[test]
    fn a_lost_line_ends_streams_with_lost_and_later_calls_were_not_sent() {
        let (conn, _core, out) = rig(&hello_line(true, 1), None).unwrap();
        let s = conn.open_stream(3);
        drop(out);
        assert_eq!(s.recv_timeout(T).unwrap(), StreamEvent::Lost);
        assert!(conn.is_closed());
        assert!(matches!(conn.call("x", json!({}), T), Err(HcError::NotSent)));
        assert!(matches!(conn.data(3, b"x"), Err(HcError::NotSent)));
        assert_eq!(conn.open_stream(9).recv_timeout(T).unwrap(), StreamEvent::Lost);
    }

    #[test]
    fn a_call_in_flight_when_the_line_dies_is_closed() {
        let (conn, mut core, out) = rig(&hello_line(true, 1), None).unwrap();
        let h = std::thread::spawn(move || conn.call("terminal.run", json!({}), T));
        read_frames(&mut core, 1);
        drop(out);
        assert!(matches!(h.join().unwrap(), Err(HcError::Closed)));
    }

    #[test]
    fn an_oversize_frame_from_the_core_drops_the_connection() {
        let (conn, _core, mut out) = rig(&hello_line(true, 1), None).unwrap();
        let mut hdr = vec![1, 0, 0, 0, 0];
        hdr.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
        out.write_all(&hdr).unwrap();
        for _ in 0..200 {
            if conn.is_closed() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(conn.is_closed());
    }
}
