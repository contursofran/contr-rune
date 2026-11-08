//! One terminal: a login shell in a PTY, parsed by alacritty_terminal's emulator.
//!
//! A reader thread feeds the PTY's bytes to the emulator and calls the Rune waker, so the
//! window redraws only when the shell printed something. The UI thread locks the emulator to
//! draw it and writes key bytes to the PTY.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use rune::Waker;

pub const SCROLLBACK: usize = 5_000;

/// The emulator's size, as alacritty_terminal wants it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize { self.rows as usize }
    fn screen_lines(&self) -> usize { self.rows as usize }
    fn columns(&self) -> usize { self.cols as usize }
}

/// What the emulator tells us while it parses: replies for the shell, title changes, exit.
#[derive(Clone)]
pub struct Events {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        match event {
            // Device status and similar queries: the shell waits for these answers.
            Event::PtyWrite(s) => {
                if let Ok(mut w) = self.writer.lock() {
                    let _ = w.write_all(s.as_bytes());
                    let _ = w.flush();
                }
            }
            Event::Title(t) => *self.title.lock().unwrap() = Some(t),
            Event::ResetTitle => *self.title.lock().unwrap() = None,
            _ => {}
        }
    }
}

pub struct Session {
    pub id: u64,
    pub cwd: PathBuf,
    pub term: Arc<Mutex<Term<Events>>>,
    title: Arc<Mutex<Option<String>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    size: Size,
    /// Set by the reader thread when there's new output; cleared by the UI after drawing.
    pub dirty: Arc<AtomicBool>,
    pub exited: Arc<AtomicBool>,
    /// Unix millis of the last output, for the "working" dot.
    pub last_output: Arc<AtomicU64>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Session {
    pub fn spawn(id: u64, cwd: &Path, size: Size, waker: Option<Waker>) -> std::io::Result<Session> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: size.rows, cols: size.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(std::io::Error::other)?;

        let mut cmd = CommandBuilder::new_default_prog();
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let mut child = pair.slave.spawn_command(cmd).map_err(std::io::Error::other)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().map_err(std::io::Error::other)?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().map_err(std::io::Error::other)?));

        let title = Arc::new(Mutex::new(None));
        let events = Events { writer: writer.clone(), title: title.clone() };
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let term = Arc::new(Mutex::new(Term::new(config, &size, events)));

        let dirty = Arc::new(AtomicBool::new(true));
        let exited = Arc::new(AtomicBool::new(false));
        let last_output = Arc::new(AtomicU64::new(now_ms()));

        // Reader: PTY bytes → emulator → wake the window.
        {
            let (term, dirty, exited, last_output) = (term.clone(), dirty.clone(), exited.clone(), last_output.clone());
            let waker = waker.clone();
            std::thread::Builder::new().name(format!("pty-{id}")).spawn(move || {
                let mut reader = reader;
                let mut parser: Processor = Processor::new();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            parser.advance(&mut *term.lock().unwrap(), &buf[..n]);
                            last_output.store(now_ms(), Ordering::Relaxed);
                            // Only wake when the UI has drawn the last batch: a flood of output
                            // collapses into one redraw per frame.
                            if !dirty.swap(true, Ordering::AcqRel)
                                && let Some(w) = &waker
                            {
                                w();
                            }
                        }
                    }
                }
                exited.store(true, Ordering::Release);
                dirty.store(true, Ordering::Release);
                if let Some(w) = &waker { w(); }
            })?;
        }

        // Reap the child so it doesn't linger as a zombie.
        std::thread::spawn(move || { let _ = child.wait(); });

        Ok(Session { id, cwd: cwd.to_path_buf(), term, title, writer, master: pair.master, size, dirty, exited, last_output })
    }

    pub fn write(&self, bytes: &[u8]) {
        if self.exited.load(Ordering::Acquire) { return; }
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
        // Typing jumps back to the bottom, as every terminal does.
        let mut t = self.term.lock().unwrap();
        if t.grid().display_offset() != 0 {
            t.scroll_display(Scroll::Bottom);
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.cols == 0 || size.rows == 0 { return; }
        self.size = size;
        let _ = self.master.resize(PtySize { rows: size.rows, cols: size.cols, pixel_width: 0, pixel_height: 0 });
        self.term.lock().unwrap().resize(size);
        self.dirty.store(true, Ordering::Release);
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().unwrap().scroll_display(Scroll::Delta(lines));
        self.dirty.store(true, Ordering::Release);
    }

    /// The tab label: the shell's OSC title, else `$ <folder>` (contr's rule for plain terminals).
    pub fn label(&self) -> String {
        if let Some(t) = self.title.lock().unwrap().as_ref().filter(|t| !t.trim().is_empty()) {
            // Shells set `user@host:~/a/b`; a tab only needs `$ b`.
            let t = t.trim();
            let path = match t.split_once(':') {
                Some((who, rest)) if who.contains('@') => rest.trim(),
                _ => t,
            };
            if path.starts_with('~') || path.starts_with('/') {
                let last = path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path);
                return format!("$ {last}");
            }
            return t.to_string();
        }
        let folder = self.cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
        format!("$ {folder}")
    }
}
