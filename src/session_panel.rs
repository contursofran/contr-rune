//! The session panel over the file tree (C4): the selected agent's todos ("Todos  N of M") and
//! artifacts ("Artifacts  N"), read from its K2 session file. Each section is hidden when empty.
//! The file is read on a thread when the selection or the file's mtime changes, only while the
//! window has the focus; the view reads the cache.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rune::{El, Tx, Waker, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::session_file::{self, Artifact, Session, Todo};
use crate::{App, paths};

/// How often the file's mtime is checked while the window has the focus.
const EVERY: Duration = Duration::from_secs(2);
/// Rows a section shows before "+N more".
const MAX_ROWS: usize = 6;

/// What one check found: the file's mtime and, when it changed, its content.
struct Read {
    id: String,
    mtime: Option<SystemTime>,
    /// `None`: the file is the same as the last read.
    session: Option<Option<Session>>,
}

pub struct SessionCache {
    id: Option<String>,
    mtime: Option<SystemTime>,
    last: Option<Instant>,
    inflight: bool,
    tx: Sender<Read>,
    rx: Receiver<Read>,
    pub session: Option<Session>,
}

impl Default for SessionCache {
    fn default() -> Self {
        let (tx, rx) = channel();
        SessionCache { id: None, mtime: None, last: None, inflight: false, tx, rx, session: None }
    }
}

/// Stat the session file of `id`; read it when its mtime is not `known` or `fresh` (a new
/// selection) asks for it. An unreadable file reads as no session.
fn check(id: &str, known: Option<SystemTime>, fresh: bool) -> Read {
    let path = paths::session_file(id);
    let mtime = path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let session = if fresh || mtime != known { Some(path.and_then(|p| session_file::read(&p).ok())) } else { None };
    Read { id: id.to_owned(), mtime, session }
}

impl SessionCache {
    /// Refresh for the selected agent `id`. Returns whether the view changes.
    pub fn poll(&mut self, id: Option<&str>, focused: bool, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(got) = self.rx.try_recv() {
            self.inflight = false;
            if self.id.as_deref() == Some(got.id.as_str()) {
                self.mtime = got.mtime;
                if let Some(session) = got.session {
                    changed |= self.session != session;
                    self.session = session;
                }
            }
        }
        let Some(id) = id else {
            self.last = None;
            self.mtime = None;
            return (self.id.take().is_some() && self.session.take().is_some()) || changed;
        };
        let fresh = self.id.as_deref() != Some(id);
        if fresh {
            self.id = Some(id.to_owned());
            self.mtime = None;
            self.last = None;
            changed |= self.session.take().is_some();
        }
        if (fresh || focused) && !self.inflight && self.last.is_none_or(|t| t.elapsed() >= EVERY) {
            self.last = Some(Instant::now());
            let (known, id) = (self.mtime, id.to_owned());
            if rune::is_headless() {
                // A headless run cannot wait for a thread.
                let _ = self.tx.send(check(&id, known, fresh));
                return self.poll(Some(&id), focused, waker, tx) || changed;
            }
            self.inflight = true;
            let (out, waker) = (self.tx.clone(), waker.cloned());
            std::thread::spawn(move || {
                let _ = out.send(check(&id, known, fresh));
                if let Some(w) = &waker {
                    w();
                }
            });
        }
        if focused {
            tx.wake_after(EVERY);
        }
        changed
    }
}

/// "now", "38m", "5h" or "2d": how long ago `then` was, for `now`, both in seconds.
pub fn age_text(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// "2 of 5": the todos that are done, of all of them.
pub fn todos_count(todos: &[Todo]) -> String {
    let done = todos.iter().filter(|t| t.status == "completed").count();
    format!("{done} of {}", todos.len())
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn section_head(k: &Kit, key: &str, title: &str, count: String) -> El {
    row()
        .key(format!("{key}/head"))
        .w_full()
        .center_y()
        .child(text(title).key("title").font(k.ty.text_xsmall).color(k.c.text_faint))
        .child(spacer())
        .child(text(count).key("count").font(k.ty.mono).color(k.c.text_muted))
}

fn more(k: &Kit, key: &str, n: usize) -> El {
    text(format!("+{n} more")).key(format!("{key}/more")).font(k.ty.text_small).color(k.c.text_faint)
}

fn todo_row(k: &Kit, i: usize, t: &Todo) -> El {
    let c = &k.c;
    let (mark, color) = match t.status.as_str() {
        "completed" => ("\u{2713}", c.text_faint),
        "in_progress" => ("\u{25cf}", c.text_strong),
        _ => ("\u{25cb}", c.text_muted),
    };
    let label = if t.status == "in_progress" && !t.active.is_empty() { &t.active } else { &t.text };
    let mut line = row()
        .key(format!("session/todo/{i}"))
        .w_full()
        .gap(6.0)
        .child(text(mark).key("mark").font(k.ty.text_small).color(color))
        .child(text(label.clone()).key("text").font(k.ty.text_small).color(color).grow(1.0));
    if t.you {
        line = line.child(text("you").key("you").font(k.ty.text_xsmall).color(c.text_faint));
    }
    line
}

fn artifact_row(k: &Kit, i: usize, a: &Artifact, now: u64) -> El {
    let key = format!("session/artifact/{i}");
    let url = a.url.clone();
    let title = if a.title.is_empty() { a.slug.as_str() } else { a.title.as_str() };
    let (name, age, faint, mono) = (title.to_owned(), age_text(now, a.updated_at), k.c.text_faint, k.ty.mono);
    Button::new(title)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .label_with(move |font, ink| {
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(text(name).font(font).color(ink).grow(1.0).min_w(0.0))
                .child(text(age).key("age").font(mono).color(faint))
        })
        .on_press_tx(move |_: &mut App, tx: &mut Tx| crate::notch::open_link(tx, &url))
        .build(k)
        .key(key)
        .w_full()
}

/// The panel for the selected agent's session; `None` when both sections are empty.
pub fn view(k: &Kit, s: &App) -> Option<El> {
    let session = s.session.session.as_ref()?;
    if session.todos.is_empty() && session.artifacts.is_empty() {
        return None;
    }
    let mut panel = col().key("session").w_full().gap(8.0).pb(8.0).px(10.0);
    if !session.todos.is_empty() {
        let mut sec = col().key("session/todos").w_full().gap(4.0).child(section_head(k, "session/todos", "Todos", todos_count(&session.todos)));
        for (i, t) in session.todos.iter().enumerate().take(MAX_ROWS) {
            sec = sec.child(todo_row(k, i, t));
        }
        if session.todos.len() > MAX_ROWS {
            sec = sec.child(more(k, "session/todos", session.todos.len() - MAX_ROWS));
        }
        panel = panel.child(sec);
    }
    if !session.artifacts.is_empty() {
        let now = unix_now();
        let mut sec = col().key("session/artifacts").w_full().gap(2.0).child(section_head(k, "session/artifacts", "Artifacts", session.artifacts.len().to_string()));
        for (i, a) in session.artifacts.iter().enumerate().take(MAX_ROWS) {
            sec = sec.child(artifact_row(k, i, a, now));
        }
        if session.artifacts.len() > MAX_ROWS {
            sec = sec.child(more(k, "session/artifacts", session.artifacts.len() - MAX_ROWS));
        }
        panel = panel.child(sec);
    }
    Some(panel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_text_table() {
        let cases = [
            (100, 100, "now"),
            (159, 100, "now"),
            (160, 100, "1m"),
            (100 + 38 * 60, 100, "38m"),
            (100 + 3599, 100, "59m"),
            (100 + 5 * 3600, 100, "5h"),
            (100 + 86_400, 100, "1d"),
            (100 + 2 * 86_400 + 5, 100, "2d"),
            (50, 100, "now"),
        ];
        for (now, then, want) in cases {
            assert_eq!(age_text(now, then), want, "{now} {then}");
        }
    }

    #[test]
    fn todos_count_is_done_of_all() {
        let t = |s: &str| Todo::new("x", "", s);
        assert_eq!(todos_count(&[t("completed"), t("in_progress"), t("pending"), t("completed")]), "2 of 4");
        assert_eq!(todos_count(&[t("pending")]), "0 of 1");
    }
}
