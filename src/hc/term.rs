//! A tab that is an hc terminal (H3, H4): the same surface as `session::Session`, fed from a framed
//! `terminal.attach` stream instead of a local PTY.
//!
//! `HcSession::start` returns at once; the thread `hc-term-<id>` dials and either calls
//! `terminal.run` for a new tab (Contract C1: empty `argv`, the C5 labels) or, for a tab restored at
//! launch (`Spec::existing`), only attaches. It attaches with the pane size and `history: 1000` and
//! feeds the bytes to `alacritty_terminal` exactly as `Session`'s reader does. Keys and paste go out
//! as DATA (kept until the attach is up), pane resizes as RESIZE, focus as FOCUS, and the client
//! sends CREDIT after each batch. `close` is `terminal.kill` then `terminal.rm`.
//!
//! H4, the stream's end decides what happens next (Contract C2):
//! - CLOSE with the reason `exited` or `killed`: the program is over, `exited` is set, the tab closes.
//! - A lost line, or CLOSE with another reason: the tab stays, `detached` is set, and the thread
//!   attaches again with `resumed: true`, waiting 0.5 s, doubled to 5 s. The emulator keeps its
//!   scrollback; the visible screen is cleared before the core's first paint.
//! - Attach errors `not_found`, `finished`, `needs_framed`: over, the tab closes. `unavailable`:
//!   wait for the `restarted` event (with a slow safety retry, 10 s, in case it was missed) (`HcSession::event`), which also re-attaches at once from
//!   any state. The `finished` and `removed` events close the tab.
//!
//! A failure before the terminal exists (no core, `spawn_failed`, ...) is reported by
//! `take_failure`; the app then replaces the tab with a local PTY.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use rune::Waker;
use serde_json::{Value, json};

use super::conn::{Conn, HcError};
use super::dial::{Dialer, Keep};
use super::framed::{FramedConn, Ledger, StreamEvent};
use super::watch::{self, TermEvent};
use crate::session::{Events, SCROLLBACK, Size, now_ms, shell_name, subtitle, tab_title};

const CALL: Duration = Duration::from_secs(10);
/// Scrollback the core replays on attach.
const HISTORY: u32 = 1000;

/// Whose terminal a tab shows. contr-rune makes plain terminal tabs (role=terminal), so it owns them:
/// closing the tab kills and removes the terminal. An agent's terminal (H8) belongs to the core and
/// its agent: closing its tab only detaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ownership {
    Owned,
    Borrowed,
}

/// What closing a tab does to its terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnClose {
    /// `terminal.kill`, then `terminal.rm`.
    KillAndRemove,
    /// A CLOSE frame on the stream; the terminal lives on.
    Detach,
}

impl Ownership {
    pub fn on_close(self) -> OnClose {
        match self {
            Ownership::Owned => OnClose::KillAndRemove,
            Ownership::Borrowed => OnClose::Detach,
        }
    }
}

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
    /// A terminal that already exists (restored at launch): attach to it instead of running one.
    pub existing: Option<String>,
    /// Whether closing the tab may kill the terminal.
    pub ownership: Ownership,
}

struct State {
    conn: Option<Arc<FramedConn>>,
    /// The attach stream's channel; 0 while there is none (channel 0 is JSON-RPC).
    channel: u32,
    address: Option<String>,
    size: Size,
    /// Key bytes written before the first attach was up.
    queued: Vec<u8>,
    /// The tab was closed (possibly before `terminal.run` returned).
    closing: bool,
    /// Bumped by each `restarted` event.
    epoch: u64,
    /// The pane has the keyboard focus (sent as FOCUS on attach and on change).
    focused: bool,
}

struct Shared {
    spec: Spec,
    state: Mutex<State>,
    /// Signalled when `epoch` or `closing` changes, or the terminal ended.
    cv: Condvar,
    /// Keeps the `hc connect` child alive.
    keep: Mutex<Option<Keep>>,
    failure: Mutex<Option<String>>,
    /// The line is lost and the thread is attaching again (the tab shows "reconnecting").
    detached: AtomicBool,
    exited: Arc<AtomicBool>,
}

impl Shared {
    fn write(&self, bytes: &[u8]) {
        let mut st = self.state.lock().unwrap();
        match st.conn.clone() {
            Some(c) if st.channel != 0 => {
                let _ = c.data(st.channel, bytes);
            }
            // Keys typed while the line is down would arrive late and out of context: drop them.
            _ if self.detached.load(Ordering::Acquire) => {}
            _ => st.queued.extend_from_slice(bytes),
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
        // An existing terminal has its address from the start (the app looks tabs up by it).
        let spec_address = spec.existing.clone();
        let exited = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            spec,
            state: Mutex::new(State { conn: None, channel: 0, address: spec_address, size, queued: Vec::new(), closing: false, epoch: 0, focused: false }),
            cv: Condvar::new(),
            keep: Mutex::new(None),
            failure: Mutex::new(None),
            detached: AtomicBool::new(false),
            exited: exited.clone(),
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
            exited,
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
            painted: AtomicBool::new(false),
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
            if let (Some(c), true) = (&st.conn, st.channel != 0) {
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

    /// The core's address of this tab's terminal, once it exists.
    pub fn address(&self) -> Option<String> {
        self.shared.state.lock().unwrap().address.clone()
    }

    /// The line to the core is lost and the tab is attaching again.
    pub fn detached(&self) -> bool {
        self.shared.detached.load(Ordering::Acquire)
    }

    /// An event for this tab's terminal: `Restarted` attaches again at once; `Finished` and
    /// `Removed` end the tab.
    pub fn event(&self, kind: TermEvent) {
        let mut st = self.shared.state.lock().unwrap();
        match kind {
            TermEvent::Restarted => st.epoch += 1,
            TermEvent::Finished | TermEvent::Removed => self.exited.store(true, Ordering::Release),
        }
        drop(st);
        self.shared.cv.notify_all();
    }

    /// The pane gained or lost the keyboard focus (FOCUS frame; sent on every attach too).
    pub fn set_focus(&self, gained: bool) {
        let mut st = self.shared.state.lock().unwrap();
        if st.focused == gained {
            return;
        }
        st.focused = gained;
        if let (Some(c), true) = (&st.conn, st.channel != 0) {
            let _ = c.focus(st.channel, gained);
        }
    }

    pub fn ownership(&self) -> Ownership {
        self.shared.spec.ownership
    }

    /// The tab was closed. An owned terminal: `terminal.kill` then `terminal.rm`, off the UI
    /// thread (when `terminal.run` is still in flight, its thread does this once it returns). A
    /// borrowed one (an agent's): only a CLOSE frame on the stream; it is never killed or removed.
    pub fn close(&self) {
        let (address, line) = {
            let mut st = self.shared.state.lock().unwrap();
            st.closing = true;
            (st.address.clone(), st.conn.clone().filter(|_| st.channel != 0).map(|c| (c, st.channel)))
        };
        self.shared.cv.notify_all();
        if self.ownership().on_close() == OnClose::Detach {
            if let Some((conn, channel)) = line {
                let _ = conn.close(channel);
            }
            return;
        }
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

/// What `create` and the re-attach hand to the stream loop.
struct Attached {
    rx: Receiver<StreamEvent>,
    channel: u32,
    conn: Arc<FramedConn>,
}

/// Why a stream loop ended.
#[derive(Debug, PartialEq)]
enum End {
    /// The tab is over (the program ended, or the tab was closed).
    Over,
    /// The line was lost (or the core closed the stream for another reason).
    Lost,
    /// A `restarted` event: attach to the new shell.
    Restart,
}

/// Why an attach did not work.
enum AttachErr {
    /// `not_found`, `finished`, `needs_framed`: the terminal is gone for this tab.
    Final,
    /// `unavailable`: wait for `restarted`.
    Unavailable,
    /// Anything else: try again after the backoff.
    Retry,
}

fn classify(e: &HcError) -> AttachErr {
    match e.kind() {
        Some("not_found" | "finished" | "needs_framed") => AttachErr::Final,
        Some("unavailable") => AttachErr::Unavailable,
        _ => AttachErr::Retry,
    }
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
    /// An attach already painted this emulator: the next one clears the screen first.
    painted: AtomicBool,
    waker: Option<Waker>,
}

impl Feed {
    fn wake(&self) {
        self.dirty.store(true, Ordering::Release);
        if let Some(w) = &self.waker {
            w();
        }
    }

    fn over(&self) -> bool {
        self.exited.load(Ordering::Acquire) || self.shared.state.lock().unwrap().closing
    }

    fn set_detached(&self, on: bool) {
        if self.shared.detached.swap(on, Ordering::AcqRel) != on {
            self.wake();
        }
    }

    fn run(self) {
        let mut next = match self.shared.spec.existing.clone() {
            None => match self.create() {
                Err(e) => {
                    *self.shared.failure.lock().unwrap() = Some(e);
                    self.wake();
                    return;
                }
                Ok(a) => a,
            },
            Some(address) => {
                self.shared.state.lock().unwrap().address = Some(address);
                self.reattach(false, true)
            }
        };
        while let Some(a) = next {
            next = match self.stream(&a) {
                End::Over => None,
                End::Lost => {
                    self.lose();
                    self.set_detached(true);
                    self.reattach(true, false)
                }
                End::Restart => {
                    self.lose();
                    self.reattach(false, true)
                }
            };
        }
        self.lose();
        self.wake();
    }

    /// The stream is gone: writes no longer have a channel.
    fn lose(&self) {
        let mut st = self.shared.state.lock().unwrap();
        st.conn = None;
        st.channel = 0;
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
        {
            let mut st = self.shared.state.lock().unwrap();
            st.address = Some(address.clone());
            st.conn = Some(conn.clone());
            if st.closing {
                drop(st);
                remove(&self.shared, &address);
                self.exited.store(true, Ordering::Release);
                return Ok(None);
            }
        }
        match self.attach_on(&conn, &address, false) {
            Ok(a) => Ok(Some(a)),
            Err(_) => {
                self.exited.store(true, Ordering::Release);
                remove(&self.shared, &address);
                Ok(None)
            }
        }
    }

    /// `terminal.attach` on `conn`, then make it the session's line: queued keys go out, and the
    /// size and focus the pane has now.
    fn attach_on(&self, conn: &Arc<FramedConn>, address: &str, resumed: bool) -> Result<Attached, AttachErr> {
        let size = self.shared.state.lock().unwrap().size;
        let mut params = json!({"terminal": address, "cols": size.cols, "rows": size.rows, "history": HISTORY});
        if resumed {
            params["resumed"] = json!(true);
        }
        let reply = conn.call("terminal.attach", params, CALL).map_err(|e| classify(&e))?;
        let channel = reply["channel"].as_u64().map(|c| c as u32).filter(|c| *c != 0).ok_or(AttachErr::Retry)?;
        let rx = conn.open_stream(channel);
        // The core paints the whole screen for a new viewer: clear ours, keep its scrollback.
        if self.painted.swap(true, Ordering::AcqRel) {
            let mut t = self.term.lock().unwrap();
            let mut blank: Processor = Processor::new();
            blank.advance(&mut *t, b"\x1b[2J\x1b[H");
        }
        let mut st = self.shared.state.lock().unwrap();
        st.conn = Some(conn.clone());
        st.channel = channel;
        let queued = std::mem::take(&mut st.queued);
        if !queued.is_empty() {
            let _ = conn.data(channel, &queued);
        }
        // The pane may have been resized while the attach was in flight.
        if st.size != size {
            let _ = conn.resize(channel, st.size.cols, st.size.rows);
        }
        if st.focused {
            let _ = conn.focus(channel, true);
        }
        drop(st);
        self.shared.detached.store(false, Ordering::Release);
        Ok(Attached { rx, channel, conn: conn.clone() })
    }

    /// Dial and attach again. `immediate`: try at once (launch, `restarted`); else wait the
    /// backoff first. Returns `None` when the tab is over.
    fn reattach(&self, mut resumed: bool, immediate: bool) -> Option<Attached> {
        // Contract C2: 0.5 s doubled to 5 s (shorter against the in-process fake).
        let (first, max) = self.shared.spec.dialer.backoff();
        let mut backoff = first;
        let mut wait = !immediate;
        loop {
            let seen = self.shared.state.lock().unwrap().epoch;
            if wait && !self.sleep(backoff, seen) {
                // A `restarted` event: the new shell, at once.
                resumed = false;
                backoff = first;
            }
            if self.over() {
                return None;
            }
            let seen = self.shared.state.lock().unwrap().epoch;
            match self.try_attach(resumed) {
                Ok(a) => {
                    self.set_detached(false);
                    return Some(a);
                }
                Err(AttachErr::Final) => {
                    self.exited.store(true, Ordering::Release);
                    return None;
                }
                Err(AttachErr::Unavailable) => {
                    self.set_detached(true);
                    // Wait for `restarted`; a slow timer tries again if the event was missed.
                    if self.park(seen)? {
                        resumed = false;
                    }
                    backoff = first;
                    wait = false;
                }
                Err(AttachErr::Retry) => {
                    self.set_detached(true);
                    wait = true;
                    backoff = (backoff * 2).min(max);
                }
            }
        }
    }

    fn try_attach(&self, resumed: bool) -> Result<Attached, AttachErr> {
        let address = self.shared.state.lock().unwrap().address.clone().ok_or(AttachErr::Final)?;
        let link = self.shared.spec.dialer.dial().map_err(|_| AttachErr::Retry)?;
        let (conn, _hello) = FramedConn::connect(link.reader, link.writer, None, CALL).map_err(|_| AttachErr::Retry)?;
        let conn = Arc::new(conn);
        let a = self.attach_on(&conn, &address, resumed)?;
        *self.shared.keep.lock().unwrap() = Some(link.keep);
        Ok(a)
    }

    /// Wait `d`; false when a `restarted` event (a new epoch than `seen`) cut the wait short.
    /// Also returns true when the tab ended, so the caller checks `over`.
    fn sleep(&self, d: Duration, seen: u64) -> bool {
        let deadline = std::time::Instant::now() + d;
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if st.epoch != seen {
                return false;
            }
            if st.closing || self.exited.load(Ordering::Acquire) {
                return true;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return true;
            }
            st = self.shared.cv.wait_timeout(st, left).unwrap().0;
        }
    }

    /// Wait for a `restarted` event (`Some(true)`), or the slow safety timer in case the event was
    /// missed (`Some(false)`); `None` when the tab ended.
    fn park(&self, seen: u64) -> Option<bool> {
        let deadline = std::time::Instant::now() + self.shared.spec.dialer.safety();
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if st.closing || self.exited.load(Ordering::Acquire) {
                return None;
            }
            if st.epoch != seen {
                return Some(true);
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Some(false);
            }
            st = self.shared.cv.wait_timeout(st, left).unwrap().0;
        }
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
    fn stream(&self, a: &Attached) -> End {
        let mut parser: Processor = Processor::new();
        let mut ledger = Ledger::default();
        let seen = self.shared.state.lock().unwrap().epoch;
        loop {
            let ev = match a.rx.recv_timeout(self.shared.spec.dialer.poll()) {
                Ok(ev) => ev,
                Err(RecvTimeoutError::Timeout) => {
                    if self.over() {
                        return End::Over;
                    }
                    if self.shared.state.lock().unwrap().epoch != seen {
                        let _ = a.conn.close(a.channel);
                        return End::Restart;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return End::Lost,
            };
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
                    // The program ended: the tab closes. Another reason (the core is stopping, the
                    // shell was lost) or an empty CLOSE: look for the terminal again.
                    return match info.reason.as_deref() {
                        Some("exited" | "killed") => {
                            self.exited.store(true, Ordering::Release);
                            End::Over
                        }
                        _ => End::Lost,
                    };
                }
                StreamEvent::Lost => return End::Lost,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::fake::Fixture;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};

    const WAIT: Duration = Duration::from_secs(5);

    fn start(fx: Value, existing: Option<&str>) -> (HcSession, Arc<Fixture>) {
        start_as(fx, existing, Ownership::Owned)
    }

    fn start_as(fx: Value, existing: Option<&str>, ownership: Ownership) -> (HcSession, Arc<Fixture>) {
        let fx = Arc::new(Fixture::from_value(fx));
        let spec = Spec {
            dialer: Dialer::Fake(fx.clone()),
            cwd: "/tmp".into(),
            project: "p".into(),
            size: Size { cols: 40, rows: 10 },
            waker: None,
            existing: existing.map(str::to_owned),
            ownership,
        };
        (HcSession::start(1, spec), fx)
    }

    fn until(what: &str, f: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !f() {
            assert!(t.elapsed() < WAIT, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn screen(s: &HcSession) -> String {
        let t = s.term.lock().unwrap();
        let g = t.grid();
        (0..g.screen_lines() as i32).map(|l| (0..g.columns()).map(|c| g[Line(l)][Column(c)].c).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn history(s: &HcSession) -> usize {
        s.term.lock().unwrap().grid().history_size()
    }

    fn live(address: &str) -> Value {
        json!({"address": address, "state": "live", "labels": {"contr": "agent", "role": "terminal"}})
    }

    fn numbered(n: usize) -> String {
        (0..n).map(|i| format!("row-{i}\r\n")).collect()
    }

    #[test]
    fn an_existing_terminal_is_attached_not_run() {
        let (s, _) = start(json!({"terminals": [live("t3")], "screens": {"t3": "hello\r\n"}}), Some("t3"));
        until("first paint", || screen(&s).contains("hello"));
        assert_eq!(s.address().as_deref(), Some("t3"));
        assert!(!s.detached() && !s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn a_lost_line_attaches_again_and_keeps_the_scrollback() {
        let (s, fx) = start(json!({"terminals": [live("t3")], "screens": {"t3": numbered(40)}}), Some("t3"));
        until("first paint", || screen(&s).contains("row-39"));
        let before = history(&s);
        assert!(before > 20, "the paint scrolled: {before}");
        fx.control(&json!({"drop": "framed", "refuse_attach_ms": 500}));
        until("detached", || s.detached());
        assert_eq!(history(&s), before, "scrollback stays while detached");
        assert!(!s.exited.load(Ordering::Acquire), "the tab stays");
        until("attached again", || !s.detached());
        until("repainted", || screen(&s).contains("row-39"));
        assert!(history(&s) >= before, "scrollback kept: {} then {}", before, history(&s));
        assert!(!s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn keys_typed_while_detached_are_dropped_not_replayed() {
        let (s, fx) = start(json!({"terminals": [live("t3")], "echo": true}), Some("t3"));
        until("attached", || s.address().is_some() && !s.detached());
        std::thread::sleep(Duration::from_millis(200));
        fx.control(&json!({"drop": "framed", "refuse_attach_ms": 500}));
        until("detached", || s.detached());
        s.write(b"lost");
        until("attached again", || !s.detached());
        std::thread::sleep(Duration::from_millis(300));
        assert!(!screen(&s).contains("lost"), "{:?}", screen(&s));
    }

    #[test]
    fn a_terminal_that_is_gone_ends_the_tab() {
        let (s, _) = start(json!({"terminals": []}), Some("t9"));
        until("exited", || s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn a_finished_terminal_ends_the_tab() {
        let mut t = live("t3");
        t["state"] = json!("finished");
        let (s, _) = start(json!({"terminals": [t]}), Some("t3"));
        until("exited", || s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn unavailable_waits_for_a_restarted_event_instead_of_retrying() {
        let fx = json!({"terminals": [live("t3")], "errors": {"terminal.attach": {"kind": "unavailable", "message": "lost, waiting for its new shell"}}});
        let (s, _) = start(fx, Some("t3"));
        until("detached", || s.detached());
        std::thread::sleep(Duration::from_millis(500));
        assert!(s.detached() && !s.exited.load(Ordering::Acquire), "parked, not given up");
        // The event wakes it; the core still says unavailable, so it parks again.
        s.event(TermEvent::Restarted);
        std::thread::sleep(Duration::from_millis(200));
        assert!(s.detached() && !s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn unavailable_without_a_restarted_event_is_retried_by_the_safety_timer() {
        let (s, fx) = start(json!({"terminals": [live("t3")], "screens": {"t3": "back\r\n"}}), Some("t3"));
        until("attached", || s.address().is_some() && !s.detached());
        std::thread::sleep(Duration::from_millis(200));
        fx.control(&json!({"drop": "framed", "refuse_attach_ms": 500, "refuse_attach_kind": "unavailable"}));
        until("detached", || s.detached());
        // No `restarted` is ever sent: the safety retry finds the core ready again.
        until("attached again", || !s.detached());
        assert!(!s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn finished_and_removed_events_set_exited() {
        for kind in [TermEvent::Finished, TermEvent::Removed] {
            let (s, _) = start(json!({"terminals": [live("t3")]}), Some("t3"));
            until("attached", || !s.detached() && s.address().is_some());
            s.event(kind);
            assert!(s.exited.load(Ordering::Acquire), "{kind:?}");
        }
    }

    #[test]
    fn a_close_with_a_reason_ends_the_tab_only_for_exit_and_kill() {
        let (s, _) = start(json!({"terminals": [live("t3")], "echo": true, "exit_on": "exit\r"}), Some("t3"));
        until("attached", || s.address().is_some());
        std::thread::sleep(Duration::from_millis(200));
        s.write(b"exit\r");
        until("exited", || s.exited.load(Ordering::Acquire));
    }

    #[test]
    fn closing_stops_the_retry_loop() {
        let (s, fx) = start(json!({"terminals": [live("t3")]}), Some("t3"));
        until("attached", || s.address().is_some());
        std::thread::sleep(Duration::from_millis(200));
        fx.control(&json!({"drop": "framed", "refuse_attach_ms": 600000}));
        until("detached", || s.detached());
        s.close();
        // The close thread kills and removes the terminal on a fresh line.
        until("removed", || fx.terminals().is_empty());
    }

    #[test]
    fn only_owned_terminals_are_killed_on_close() {
        assert_eq!(Ownership::Owned.on_close(), OnClose::KillAndRemove);
        assert_eq!(Ownership::Borrowed.on_close(), OnClose::Detach);
    }

    #[test]
    fn closing_a_borrowed_tab_detaches_and_leaves_the_terminal_live() {
        let (s, fx) = start_as(json!({"terminals": [live("t3")]}), Some("t3"), Ownership::Borrowed);
        until("attached", || s.address().is_some() && !s.detached());
        std::thread::sleep(Duration::from_millis(200));
        s.close();
        std::thread::sleep(Duration::from_millis(600));
        let t = fx.terminals();
        assert_eq!(t.len(), 1, "not removed: {t:?}");
        assert_eq!(t[0]["state"], "live", "not killed: {t:?}");
    }

    #[test]
    fn closing_an_owned_tab_kills_and_removes() {
        let (s, fx) = start_as(json!({"terminals": [live("t3")]}), Some("t3"), Ownership::Owned);
        until("attached", || s.address().is_some() && !s.detached());
        s.close();
        until("removed", || fx.terminals().is_empty());
    }
}
