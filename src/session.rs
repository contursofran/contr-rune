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

impl Events {
    /// Replies for the program go to `writer`; the program's title lands in `title`.
    pub fn new(writer: Arc<Mutex<Box<dyn Write + Send>>>, title: Arc<Mutex<Option<String>>>) -> Events {
        Events { writer, title }
    }
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
    pub term: Arc<Mutex<Term<Events>>>,
    title: Arc<Mutex<Option<String>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    size: Size,
    /// The folder the shell started in: the subtitle until the shell's title names another.
    cwd: PathBuf,
    /// Set by the reader thread when there's new output; cleared by the UI after drawing.
    pub dirty: Arc<AtomicBool>,
    pub exited: Arc<AtomicBool>,
    /// Unix millis of the last output, for the "working" dot.
    pub last_output: Arc<AtomicU64>,
    /// Counts every change of the screen's content (output, resize); the find bar's cached matches
    /// are valid while it stays the same.
    pub generation: Arc<AtomicU64>,
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
        let events = Events::new(writer.clone(), title.clone());
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let term = Arc::new(Mutex::new(Term::new(config, &size, events)));

        let dirty = Arc::new(AtomicBool::new(true));
        let exited = Arc::new(AtomicBool::new(false));
        let last_output = Arc::new(AtomicU64::new(now_ms()));
        let generation = Arc::new(AtomicU64::new(0));

        // Reader: PTY bytes → emulator → wake the window.
        {
            let (term, dirty, exited, last_output, generation) = (term.clone(), dirty.clone(), exited.clone(), last_output.clone(), generation.clone());
            let waker = waker.clone();
            std::thread::Builder::new().name(format!("pty-{id}")).spawn(move || {
                let mut reader = reader;
                let mut parser: Processor = Processor::new();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            {
                                let mut t = term.lock().unwrap();
                                parser.advance(&mut *t, &buf[..n]);
                                generation.fetch_add(1, Ordering::AcqRel);
                            }
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

        Ok(Session { id, term, title, writer, master: pair.master, size, cwd: cwd.to_path_buf(), dirty, exited, last_output, generation })
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
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.dirty.store(true, Ordering::Release);
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().unwrap().scroll_display(Scroll::Delta(lines));
        self.dirty.store(true, Ordering::Release);
    }

    /// The tab title: the program name from the shell's OSC title (see [`tab_title`]).
    pub fn tab_title(&self) -> String {
        tab_title(self.title.lock().unwrap().as_deref(), &shell_name())
    }

    /// The row's second line (see [`subtitle`]); `None` when there is nothing useful to show.
    pub fn subtitle(&self) -> Option<String> {
        let home = std::env::var("HOME").ok();
        let title = self.tab_title();
        let osc = self.title.lock().unwrap().clone();
        subtitle(osc.as_deref(), &shell_name(), &self.cwd.to_string_lossy(), home.as_deref(), &title)
    }
}

/// The name of the user's shell (`$SHELL`'s last path part), `shell` when unknown.
pub fn shell_name() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.trim_end_matches('/').rsplit('/').next().map(str::to_owned))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "shell".into())
}

/// A tab's title from the shell's OSC title: control characters stripped; `user@host:~/a/b` gives
/// `b`; an empty title gives `fallback` (the shell's name).
pub fn tab_title(osc: Option<&str>, fallback: &str) -> String {
    let clean: String = osc.unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let t = clean.trim();
    let path = match t.split_once(':') {
        Some((who, rest)) if who.contains('@') => rest.trim(),
        _ => t,
    };
    let out = if path.starts_with('~') || path.starts_with('/') {
        path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path)
    } else {
        t
    };
    if out.is_empty() { fallback.to_owned() } else { out.to_owned() }
}

/// A path shortened for a narrow row: `$HOME` becomes `~`; a deep path keeps its root and last
/// folder (`~/a/b/c` gives `~/…/c`, `/usr/local/bin` gives `/…/bin`).
pub fn shorten_path(path: &str, home: Option<&str>) -> String {
    let path = path.trim_end_matches('/');
    let path = match home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty()) {
        Some(h) if path == h => "~".to_owned(),
        Some(h) => match path.strip_prefix(h).filter(|r| r.starts_with('/')) {
            Some(rest) => format!("~{rest}"),
            None => path.to_owned(),
        },
        None => path.to_owned(),
    };
    if path.is_empty() {
        return "/".into();
    }
    let (root, rest) = match path.strip_prefix('~') {
        Some(r) => ("~", r),
        None => ("", path.as_str()),
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    let limit = if root.is_empty() { 2 } else { 1 };
    if parts.len() > limit { format!("{root}/…/{}", parts[parts.len() - 1]) } else { path }
}

/// A terminal row's second line, never a copy of its `title`: the shortened folder from a
/// `user@host:path` or path OSC title; else the shell's name while the title is a running
/// program; else, with no title yet, the folder the shell started in.
pub fn subtitle(osc: Option<&str>, shell: &str, start: &str, home: Option<&str>, title: &str) -> Option<String> {
    let clean: String = osc.unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let t = clean.trim();
    let path = match t.split_once(':') {
        Some((who, rest)) if who.contains('@') => rest.trim(),
        _ => t,
    };
    let out = if path.starts_with('~') || path.starts_with('/') {
        shorten_path(path, home)
    } else if t.is_empty() {
        shorten_path(start, home)
    } else {
        shell.to_owned()
    };
    Some(out).filter(|o| !o.is_empty() && o != title)
}

#[cfg(test)]
mod tests {
    use super::{shorten_path, subtitle, tab_title};

    #[test]
    fn paths_are_shortened() {
        let home = Some("/Users/me");
        assert_eq!(shorten_path("/Users/me", home), "~");
        assert_eq!(shorten_path("/Users/me/", home), "~");
        assert_eq!(shorten_path("/Users/me/a", home), "~/a");
        assert_eq!(shorten_path("/Users/me/a/b", home), "~/…/b");
        assert_eq!(shorten_path("~/a/b/c", home), "~/…/c");
        assert_eq!(shorten_path("/Users/mellow/a", home), "/…/a");
        assert_eq!(shorten_path("/Users/mellow", home), "/Users/mellow");
        assert_eq!(shorten_path("/usr", None), "/usr");
        assert_eq!(shorten_path("/usr/local", None), "/usr/local");
        assert_eq!(shorten_path("/usr/local/bin", None), "/…/bin");
        assert_eq!(shorten_path("/", None), "/");
    }

    #[test]
    fn subtitle_is_never_the_title() {
        let home = Some("/Users/me");
        assert_eq!(subtitle(Some("me@box:~/a/b"), "zsh", "/x", home, "b").as_deref(), Some("~/…/b"));
        assert_eq!(subtitle(Some("~/a"), "zsh", "/x", home, "a").as_deref(), Some("~/a"));
        // A program: the shell's name, unless that is the title.
        assert_eq!(subtitle(Some("go test ./..."), "zsh", "/x", home, "go test ./...").as_deref(), Some("zsh"));
        assert_eq!(subtitle(Some("zsh"), "zsh", "/x", home, "zsh"), None);
        // No title yet: where the shell started.
        assert_eq!(subtitle(None, "zsh", "/Users/me/proj/app", home, "zsh").as_deref(), Some("~/…/app"));
        assert_eq!(subtitle(None, "zsh", "/", home, "zsh").as_deref(), Some("/"));
        assert_eq!(subtitle(Some("~"), "zsh", "/x", home, "~"), None);
    }

    #[test]
    fn empty_title_falls_back_to_the_shell() {
        assert_eq!(tab_title(None, "zsh"), "zsh");
        assert_eq!(tab_title(Some(""), "zsh"), "zsh");
        assert_eq!(tab_title(Some("  \u{7}\n "), "zsh"), "zsh");
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(tab_title(Some("go\u{1b}[0m test\u{7} ./..."), "zsh"), "go[0m test ./...");
        assert_eq!(tab_title(Some("vim\r\n"), "zsh"), "vim");
    }

    #[test]
    fn user_host_path_is_the_last_folder() {
        assert_eq!(tab_title(Some("me@box:~/a/b"), "zsh"), "b");
        assert_eq!(tab_title(Some("me@box:~/a/b/"), "zsh"), "b");
        assert_eq!(tab_title(Some("/usr/local"), "zsh"), "local");
        assert_eq!(tab_title(Some("~"), "zsh"), "~");
    }

    #[test]
    fn programs_keep_their_arguments() {
        assert_eq!(tab_title(Some("go test ./..."), "zsh"), "go test ./...");
        assert_eq!(tab_title(Some("zsh"), "bash"), "zsh");
    }
}
