//! What a tab runs: a local PTY (`Session`) or an hc terminal (`HcSession`). Tabs, split, find,
//! the sidebar and the diff only see this one surface.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use alacritty_terminal::term::Term;

use crate::hc::term::HcSession;
use crate::hc::watch::TermEvent;
use crate::session::{Events, Session, Size};

pub enum Pty {
    Local(Session),
    Hc(HcSession),
}

impl Pty {
    pub fn id(&self) -> u64 {
        match self {
            Pty::Local(s) => s.id,
            Pty::Hc(s) => s.id,
        }
    }

    pub fn term(&self) -> &Arc<Mutex<Term<Events>>> {
        match self {
            Pty::Local(s) => &s.term,
            Pty::Hc(s) => &s.term,
        }
    }

    pub fn dirty(&self) -> &AtomicBool {
        match self {
            Pty::Local(s) => &s.dirty,
            Pty::Hc(s) => &s.dirty,
        }
    }

    pub fn exited(&self) -> &AtomicBool {
        match self {
            Pty::Local(s) => &s.exited,
            Pty::Hc(s) => &s.exited,
        }
    }

    pub fn last_output(&self) -> &AtomicU64 {
        match self {
            Pty::Local(s) => &s.last_output,
            Pty::Hc(s) => &s.last_output,
        }
    }

    pub fn generation(&self) -> &AtomicU64 {
        match self {
            Pty::Local(s) => &s.generation,
            Pty::Hc(s) => &s.generation,
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        match self {
            Pty::Local(s) => s.write(bytes),
            Pty::Hc(s) => s.write(bytes),
        }
    }

    pub fn resize(&mut self, size: Size) {
        match self {
            Pty::Local(s) => s.resize(size),
            Pty::Hc(s) => s.resize(size),
        }
    }

    pub fn scroll(&self, lines: i32) {
        match self {
            Pty::Local(s) => s.scroll(lines),
            Pty::Hc(s) => s.scroll(lines),
        }
    }

    pub fn tab_title(&self) -> String {
        match self {
            Pty::Local(s) => s.tab_title(),
            Pty::Hc(s) => s.tab_title(),
        }
    }

    pub fn subtitle(&self) -> Option<String> {
        match self {
            Pty::Local(s) => s.subtitle(),
            Pty::Hc(s) => s.subtitle(),
        }
    }

    /// An hc tab is attaching again after a lost line (its pane shows "reconnecting").
    pub fn detached(&self) -> bool {
        matches!(self, Pty::Hc(s) if s.detached())
    }

    /// The pane gained or lost the keyboard focus (an hc terminal gets a FOCUS frame).
    pub fn set_focus(&self, gained: bool) {
        if let Pty::Hc(s) = self {
            s.set_focus(gained);
        }
    }

    /// The core's address of this tab's terminal (hc tabs only, once it exists).
    pub fn address(&self) -> Option<String> {
        match self {
            Pty::Hc(s) => s.address(),
            Pty::Local(_) => None,
        }
    }

    /// An event from the core for this tab's terminal (hc tabs only).
    pub fn hc_event(&self, address: &str, kind: TermEvent) {
        if let Pty::Hc(s) = self
            && s.address().as_deref() == Some(address)
        {
            s.event(kind);
        }
    }

    /// The tab is closed: an hc terminal is killed and removed (a local PTY ends when dropped).
    pub fn close(&self) {
        if let Pty::Hc(s) = self {
            s.close();
        }
    }
}
