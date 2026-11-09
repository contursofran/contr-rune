//! A tab that is an hc terminal (H3): the same surface as `session::Session`, fed from a framed
//! `terminal.attach` stream instead of a local PTY.
//!
//! `HcSession::start` returns at once; the thread `hc-term-<id>` dials, calls `terminal.run`
//! (Contract C1: empty `argv`, the C5 labels), attaches with the pane size and `history: 1000`,
//! and feeds the bytes to `alacritty_terminal` exactly as `Session`'s reader does. Keys and
//! paste go out as DATA (kept until the attach is up), pane resizes as RESIZE, and the client
//! sends CREDIT after each batch. `close` is `terminal.kill` then `terminal.rm`. A stream CLOSE
//! with a reason (the shell exited, the terminal was killed) sets `exited`, so the tab closes.
//!
//! A failure before the terminal exists (no core, `spawn_failed`, ...) is reported by
//! `take_failure`; the app then replaces the tab with a local PTY. Re-attaching after a lost line
//! is H4: here a lost line only stops the feed.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use rune::Waker;
use serde_json::{Value, json};

use super::conn::{Conn, HcError};
use super::dial::{Dialer, Keep};
use super::framed::{FramedConn, Ledger, StreamEvent};
use super::watch;
use crate::session::{Events, SCROLLBACK, Size, now_ms, shell_name, subtitle, tab_title};

const CALL: Duration = Duration::from_secs(10);
/// Scrollback the core replays on attach.
const HISTORY: u32 = 1000;

/// What a new hc tab needs.
#[derive(Clone)]
pub struct Spec {
    pub dialer: Dialer,
    /// The project's folder: the shell starts here and `worktree` is this path.
    pub cwd: PathBuf,
    /// The project's folder name (label `project`).
    pub project: String,
    pub size: Size,
    pub waker: Option<Waker>,
}

struct State {
    conn: Option<Arc<FramedConn>>,
    channel: u32,
    address: Option<String>,
    size: Size,
    /// Key bytes written before the attach was up.
    queued: Vec<u8>,
    /// The tab was closed (possibly before `terminal.run` returned).
    closing: bool,
}

struct Shared {
    spec: Spec,
    state: Mutex<State>,
    /// Keeps the `hc connect` child alive.
    keep: Mutex<Option<Keep>>,
    failure: Mutex<Option<String>>,
}

impl Shared {
    fn write(&self, bytes: &[u8]) {
        let mut st = self.state.lock().unwrap();
        match st.conn.clone() {
            Some(c) => {
                let _ = c.data(st.channel, bytes);
            }
            None => st.queued.extend_from_slice(bytes),
        }
    }
}

/// The emulator's reply channel (`Events`): replies go out as DATA.
struct Replies(Arc<Shared>);

impl Write for Replies {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.write(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct HcSession {
    pub id: u64,
    pub term: Arc<Mutex<Term<Events>>>,
    title: Arc<Mutex<Option<String>>>,
    shared: Arc<Shared>,
    size: Size,
    pub dirty: Arc<AtomicBool>,
    pub exited: Arc<AtomicBool>,
    pub last_output: Arc<AtomicU64>,
    pub generation: Arc<AtomicU64>,
}

impl HcSession {
    pub fn start(id: u64, spec: Spec) -> HcSession {
        let size = spec.size;
        let waker = spec.waker.clone();
        let shared = Arc::new(Shared {
            spec,
            state: Mutex::new(State { conn: None, channel: 0, address: None, size, queued: Vec::new(), closing: false }),
            keep: Mutex::new(None),
            failure: Mutex::new(None),
        });
        let title = Arc::new(Mutex::new(None));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(Replies(shared.clone()))));
        let events = Events::new(writer, title.clone());
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let term = Arc::new(Mutex::new(Term::new(config, &size, events)));
        let s = HcSession {
            id,
            term,
            title,
            shared,
            size,
            dirty: Arc::new(AtomicBool::new(true)),
            exited: Arc::new(AtomicBool::new(false)),
            last_output: Arc::new(AtomicU64::new(now_ms())),
            generation: Arc::new(AtomicU64::new(0)),
        };
        let feed = Feed {
            id,
            shared: s.shared.clone(),
            term: s.term.clone(),
            dirty: s.dirty.clone(),
            exited: s.exited.clone(),
            last_output: s.last_output.clone(),
            generation: s.generation.clone(),
            waker,
        };
        if let Err(e) = std::thread::Builder::new().name(format!("hc-term-{id}")).spawn(move || feed.run()) {
            *s.shared.failure.lock().unwrap() = Some(e.to_string());
        }
        s
    }

    pub fn write(&self, bytes: &[u8]) {
        if self.exited.load(Ordering::Acquire) {
            return;
        }
        self.shared.write(bytes);
        // Typing jumps back to the bottom, as every terminal does.
        let mut t = self.term.lock().unwrap();
        if t.grid().display_offset() != 0 {
            t.scroll_display(Scroll::Bottom);
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        {
            let mut st = self.shared.state.lock().unwrap();
            st.size = size;
            if let Some(c) = &st.conn {
                let _ = c.resize(st.channel, size.cols, size.rows);
            }
        }
        self.term.lock().unwrap().resize(size);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.dirty.store(true, Ordering::Release);
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().unwrap().scroll_display(Scroll::Delta(lines));
        self.dirty.store(true, Ordering::Release);
    }

    pub fn tab_title(&self) -> String {
        tab_title(self.title.lock().unwrap().as_deref(), &shell_name())
    }

    pub fn subtitle(&self) -> Option<String> {
        let home = std::env::var("HOME").ok();
        let title = self.tab_title();
        let osc = self.title.lock().unwrap().clone();
        subtitle(osc.as_deref(), &shell_name(), &self.shared.spec.cwd.to_string_lossy(), home.as_deref(), &title)
    }

    /// Why the terminal could not be created (once): the app falls back to a local PTY.
    pub fn take_failure(&self) -> Option<String> {
        self.shared.failure.lock().unwrap().take()
    }

    /// The tab was closed: `terminal.kill` then `terminal.rm`, off the UI thread. When
    /// `terminal.run` is still in flight, its thread does this once it returns.
    pub fn close(&self) {
        let address = {
            let mut st = self.shared.state.lock().unwrap();
            st.closing = true;
            st.address.clone()
        };
        if let Some(address) = address {
            let shared = self.shared.clone();
            let _ = std::thread::Builder::new().name("hc-term-close".into()).spawn(move || remove(&shared, &address));
        }
    }
}

/// Kill and remove `address`, on the session's connection while it lives, else on a fresh one.
/// Errors are ignored: a terminal that already finished still gets removed.
fn remove(shared: &Shared, address: &str) {
    let call = |method: &str| -> Result<Value, HcError> {
        let conn = shared.state.lock().unwrap().conn.clone().filter(|c| !c.is_closed());
        match conn {
            Some(c) => c.call(method, json!({"terminal": address}), CALL),
            None => {
                let link = shared.spec.dialer.dial().map_err(HcError::Io)?;
                let conn = Conn::new(link.reader, link.writer, None);
                watch::handshake(&conn, false).map_err(|e| HcError::Io(io::Error::other(e)))?;
                let r = conn.call(method, json!({"terminal": address}), CALL);
                drop(link.keep);
                r
            }
        }
    };
    let _ = call("terminal.kill");
    let _ = call("terminal.rm");
}

/// What `create` hands to the stream loop.
struct Attached {
    rx: Receiver<StreamEvent>,
    channel: u32,
    conn: Arc<FramedConn>,
}

/// The session thread's state.
struct Feed {
    id: u64,
    shared: Arc<Shared>,
    term: Arc<Mutex<Term<Events>>>,
    dirty: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    last_output: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    waker: Option<Waker>,
}

impl Feed {
    fn wake(&self) {
        self.dirty.store(true, Ordering::Release);
        if let Some(w) = &self.waker {
            w();
        }
    }

    fn run(self) {
        match self.create() {
            Err(e) => *self.shared.failure.lock().unwrap() = Some(e),
            Ok(Some(a)) => self.stream(a),
            Ok(None) => {}
        }
        self.wake();
    }

    /// Dial, `terminal.run`, `terminal.attach`. `Err`: no terminal was made (the app falls back
    /// to a local PTY). `Ok(None)`: the terminal exists but the tab is over (closed meanwhile,
    /// or the attach failed): `exited` is set and the terminal is removed.
    fn create(&self) -> Result<Option<Attached>, String> {
        let link = self.shared.spec.dialer.dial().map_err(|e| e.to_string())?;
        let (conn, _hello) = FramedConn::connect(link.reader, link.writer, None, CALL).map_err(|e| e.to_string())?;
        *self.shared.keep.lock().unwrap() = Some(link.keep);
        let conn = Arc::new(conn);
        let address = self.run_terminal(&conn)?;
        // From here on the terminal exists: a failure ends the tab instead of falling back.
        let size = {
            let mut st = self.shared.state.lock().unwrap();
            st.address = Some(address.clone());
            st.conn = Some(conn.clone());
            if st.closing {
                drop(st);
                remove(&self.shared, &address);
                self.exited.store(true, Ordering::Release);
                return Ok(None);
            }
            st.size
        };
        let attached = conn.call("terminal.attach", json!({"terminal": address, "cols": size.cols, "rows": size.rows, "history": HISTORY}), CALL);
        let Some(channel) = attached.as_ref().ok().and_then(|r| r["channel"].as_u64()).map(|c| c as u32) else {
            self.exited.store(true, Ordering::Release);
            remove(&self.shared, &address);
            return Ok(None);
        };
        let rx = conn.open_stream(channel);
        {
            let mut st = self.shared.state.lock().unwrap();
            st.channel = channel;
            let queued = std::mem::take(&mut st.queued);
            if !queued.is_empty() {
                let _ = conn.data(channel, &queued);
            }
            // The pane may have been resized while the attach was in flight.
            if st.size != size {
                let _ = conn.resize(channel, st.size.cols, st.size.rows);
            }
        }
        Ok(Some(Attached { rx, channel, conn }))
    }

    /// `terminal.run` for a new tab; a taken name is retried with another suffix.
    fn run_terminal(&self, conn: &FramedConn) -> Result<String, String> {
        let spec = &self.shared.spec;
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let size = self.shared.state.lock().unwrap().size;
        let mut last = String::new();
        for attempt in 0..4u64 {
            let name = format!("term-{:02}{:02}{}", (secs / 60) % 60, secs % 60, self.id + attempt * 1000);
            let params = json!({
                "argv": [],
                "cwd": spec.cwd,
                "name": name,
                "labels": {"contr": "agent", "role": "terminal", "project": spec.project, "worktree": spec.cwd},
                "no_parent": true,
                "outlive_parent": true,
                "wait": false,
                "size": {"cols": size.cols, "rows": size.rows},
                "env": {"TERM": "xterm-256color", "COLORTERM": "truecolor"}
            });
            match conn.call("terminal.run", params, CALL) {
                Ok(v) => return v["address"].as_str().map(str::to_owned).ok_or_else(|| "terminal.run: no address".to_owned()),
                Err(e) if e.kind() == Some("name_taken") => last = e.to_string(),
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(last)
    }

    /// Stream bytes into the emulator until the stream ends.
    fn stream(&self, a: Attached) {
        let mut parser: Processor = Processor::new();
        let mut ledger = Ledger::default();
        for ev in a.rx {
            match ev {
                StreamEvent::Data(d) => {
                    {
                        let mut t = self.term.lock().unwrap();
                        parser.advance(&mut *t, &d);
                        self.generation.fetch_add(1, Ordering::AcqRel);
                    }
                    self.last_output.store(now_ms(), Ordering::Relaxed);
                    ledger.consumed(d.len());
                    while let Some(n) = ledger.take() {
                        let _ = a.conn.credit(a.channel, n);
                    }
                    // Only wake when the UI has drawn the last batch (see `Session`).
                    if !self.dirty.swap(true, Ordering::AcqRel)
                        && let Some(w) = &self.waker
                    {
                        w();
                    }
                }
                StreamEvent::Resize { .. } => {}
                StreamEvent::Close(info) => {
                    // An empty CLOSE answers our own detach; a reason means the program ended.
                    if !info.is_ack() {
                        self.exited.store(true, Ordering::Release);
                    }
                    break;
                }
                StreamEvent::Lost => break,
            }
        }
    }
}
