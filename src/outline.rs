//! The orchestrator outline (H10): a column of short ticks on the right edge of an orchestrator's
//! terminal, one per open to-do of its project (oldest first, at most 40), with the count of new
//! ones above. Hovering a tick shows a card at its left (headline, detail, clock time). Pressing a
//! tick marks it and the ones before it read and keeps its card open (hc has no scroll-to).
//!
//! "New" is a tick that came while the owner was away: a terminal starts with what is pinned as old,
//! and while it is shown in a focused window everything counts as read.
//!
//! Keys: with the terminal in front, ⌥⇥ moves focus to the first tick (plain ⇥ stays the shell's);
//! then ↑ ↓ move between ticks, ⏎ or space opens one and Esc gives focus back to the terminal.
//! A click on a tick opens it but does not keep focus: the next keys go to the shell.
//! Rune does the focus work (a `list` focus group of `focusable` ticks); `key` only enters and leaves.
//!
//! Elements: `outline` (label "Orchestrator messages, N new"), `outline/count`, `outline/tick/<ID>`
//! (a 20x10 hit area; the label is the headline, plus ", new"), `outline/card`, `outline/card/title`,
//! `outline/card/detail`, `outline/card/time`.

use std::collections::{HashMap, HashSet};

use rune::{Color, El, Key, KeyEvent, Tx, col, el, row, text};
use rune_kit::{Kit, Placement, popover};

use crate::App;
use crate::hc::owner::Todo;
use crate::theme::{META, RADIUS_12, ROW_SMALL};

/// Most ticks shown (the newest).
const MAX_TICKS: usize = 40;
/// A tick's hit area, and the width of the hovered bar.
const HIT_W: f32 = 20.0;
const HIT_H: f32 = 10.0;
const CARD_W: f32 = 380.0;

/// One tick: a to-do as the outline shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub headline: String,
    pub detail: String,
    /// "14:38", the time of day the to-do was pinned; empty when unknown.
    pub time: String,
    /// The bar's width when not hovered.
    pub width: u32,
}

/// The tick of `t`: headline = first non-empty line; detail = the next non-empty line, else
/// "to do"/"to know" and " · from <from>"; width `min(14, 8 + chars / 30)`.
pub fn message(t: &Todo) -> Message {
    let mut lines = t.text.lines().map(str::trim).filter(|l| !l.is_empty());
    let headline = lines.next().unwrap_or_default().to_owned();
    let detail = lines.next().map(str::to_owned).unwrap_or_else(|| {
        let kind = match t.kind.as_str() {
            "know" => "to know",
            "do" => "to do",
            other => other,
        };
        match (kind.is_empty(), t.from.is_empty()) {
            (true, false) => format!("from {}", t.from),
            (false, false) => format!("{kind} · from {}", t.from),
            _ => kind.to_owned(),
        }
    });
    let width = (8 + t.text.chars().count() / 30).min(14) as u32;
    Message { id: t.id.clone(), headline, detail, time: clock(&t.pinned), width }
}

/// "HH:MM" of an RFC 3339 time as written (`2026-11-08T14:38:00-03:00` gives `14:38`); empty when
/// it does not look like one.
fn clock(rfc3339: &str) -> String {
    let hm = rfc3339.get(11..16).filter(|hm| hm.bytes().enumerate().all(|(i, b)| if i == 2 { b == b':' } else { b.is_ascii_digit() }));
    hm.unwrap_or_default().to_owned()
}

/// What the outline remembers: which ticks each orchestrator tab has shown to the owner, the hovered
/// tick and the one a press keeps open.
#[derive(Default)]
pub struct State {
    seen: HashMap<u64, HashSet<String>>,
    hover: Option<String>,
    open: Option<String>,
    /// The plain ⇥ that ⌥⇥ sent on is on its way to Rune's focus order, not to the shell.
    entering: bool,
    /// ⏎ or space was just pressed on a tick: the press that follows is the keyboard's, not a click.
    keyed: bool,
}

impl State {
    /// Note the ticks `ids` of tab `tab`. A tab met for the first time counts them old; one the owner
    /// is looking at (`looking`) reads them all; otherwise new ones stay new.
    fn observe(&mut self, tab: u64, ids: &[&str], looking: bool) {
        let now = || ids.iter().map(|i| (*i).to_owned()).collect();
        match self.seen.get_mut(&tab) {
            None => {
                self.seen.insert(tab, now());
            }
            Some(seen) if looking => *seen = now(),
            Some(_) => {}
        }
    }

    fn is_new(&self, tab: u64, id: &str) -> bool {
        self.seen.get(&tab).is_some_and(|s| !s.contains(id))
    }
}

/// The shown orchestrator tab and its ticks, `None` for any other terminal or with no to-dos.
fn current(s: &App) -> Option<(u64, Vec<Message>)> {
    let tab = s.active?;
    let agent = s.tab_agent(tab).filter(|a| a.is_orchestrator())?;
    let project = s.projects.get(agent.project?)?;
    let all = s.owner_state.open_todos(&project.name);
    let msgs: Vec<Message> = all[all.len().saturating_sub(MAX_TICKS)..].iter().map(|t| message(t)).collect();
    (!msgs.is_empty()).then_some((tab, msgs))
}

impl App {
    /// Once per frame, before the view: tell the outline what each orchestrator tab holds.
    pub fn sync_outline(&mut self) {
        let looking = self.focused.then_some(self.active).flatten();
        let held: Vec<(u64, Vec<String>)> = self
            .tabs
            .iter()
            .map(|t| t.session.id())
            .filter_map(|id| {
                let agent = self.tab_agent(id).filter(|a| a.is_orchestrator())?;
                let project = agent.project.and_then(|p| self.projects.get(p))?;
                Some((id, self.owner_state.open_todos(&project.name).iter().map(|t| t.id.clone()).collect()))
            })
            .collect();
        for (id, todo_ids) in held {
            let refs: Vec<&str> = todo_ids.iter().map(String::as_str).collect();
            self.outline.observe(id, &refs, looking == Some(id));
        }
    }

    /// A press on tick `id`: it and the ticks before it are read; its card stays open (a second
    /// press closes it).
    fn press_tick(&mut self, id: &str) {
        if let Some((tab, msgs)) = current(self) {
            let upto = msgs.iter().position(|m| m.id == id).map_or(0, |i| i + 1);
            self.outline.seen.entry(tab).or_default().extend(msgs[..upto].iter().map(|m| m.id.clone()));
        }
        self.outline.open = if self.outline.open.as_deref() == Some(id) { None } else { Some(id.to_owned()) };
    }
}

/// Keys of the outline, from the window's `on_key`: `Some(used)` when the outline decides the key,
/// `None` to go on. ⌥⇥ sends a plain ⇥ to Rune's focus order (the ticks come first in it) and that
/// ⇥ is let by, not written to the shell; Esc with a tick focused blurs it, so the next key reaches
/// the shell.
pub fn key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    let plain = ev.mods == Default::default();
    match ev.key {
        Key::Escape if plain && tx.focused().is_some_and(|f| f.starts_with("outline/tick/")) => {
            tx.blur();
            Some(true)
        }
        Key::Tab if plain && s.outline.entering => {
            s.outline.entering = false;
            Some(false)
        }
        Key::Tab if ev.mods.alt && !(ev.mods.ctrl || ev.mods.cmd || ev.mods.shift) && current(s).is_some() => {
            // With a control focused Rune's focus order takes the sent ⇥ itself; with none it
            // reaches `on_key` first, which must let it by.
            s.outline.entering = tx.focused().is_none();
            tx.send_key(KeyEvent { key: Key::Tab, mods: Default::default() });
            Some(true)
        }
        _ => None,
    }
}

/// The column of ticks, pinned to the right edge and centred on the height; `None` when the shown
/// terminal is not an orchestrator's or has no ticks.
pub fn ticks(k: &Kit, s: &App) -> Option<El> {
    let (tab, msgs) = current(s)?;
    let c = &k.c;
    let new = msgs.iter().filter(|m| s.outline.is_new(tab, &m.id)).count();
    let mut column = col().key("outline").role("list").focus_group().on_key(|s: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
        s.outline.keyed = matches!(&ev.key, Key::Enter) || ev.key == Key::Text(" ".into());
        false
    }).label(format!("Orchestrator messages, {new} new")).items_end().gap(1.0).py(6.0);
    if new > 0 {
        column = column.child(text(new.to_string()).key("outline/count").font(META).color(c.text_base).mb(2.0));
    }
    for m in msgs {
        let is_new = s.outline.is_new(tab, &m.id);
        let hovered = s.outline.hover.as_deref() == Some(m.id.as_str());
        let width = if hovered { HIT_W } else { m.width as f32 };
        let ink = if is_new || hovered { c.text_base } else { k.alpha(c.text_base, 0.14) };
        let label = if is_new { format!("{}, new", m.headline) } else { m.headline.clone() };
        let (enter, press) = (m.id.clone(), m.id.clone());
        column = column.child(
            col()
                .key(format!("outline/tick/{}", m.id))
                .role("button")
                .label(label)
                .focusable(true)
                .tab_index(1)
                .focus_ring(2.0, 0.0, c.border_focus)
                .w(HIT_W)
                .h(HIT_H)
                .justify_center()
                .items_end()
                .fill(Color::TRANSPARENT)
                .on_hover_change(move |s: &mut App, inside, _tx| {
                    if inside {
                        s.outline.hover = Some(enter.clone());
                    } else if s.outline.hover.as_deref() == Some(enter.as_str()) {
                        s.outline.hover = None;
                    }
                })
                .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    // A click leaves focus with the terminal; only ⏎ and space keep it on the tick.
                    if !std::mem::take(&mut s.outline.keyed) {
                        tx.blur();
                    }
                    s.press_tick(&press);
                })
                .child(el().w(width).h(2.0).radius(1.0).fill(ink)),
        );
    }
    Some(col().pin(None, Some(0.0), Some(10.0), Some(0.0)).justify_center().items_end().child(column))
}

/// Push the card of the hovered (else the kept open) tick onto the overlay `layer`.
pub fn card(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let Some((_, msgs)) = current(s) else { return };
    let shown = s.outline.hover.as_deref().or(s.outline.open.as_deref());
    let Some(m) = msgs.iter().find(|m| Some(m.id.as_str()) == shown) else { return };
    let c = &k.c;
    let mut second = row().w_full().gap(12.0).child(text(m.detail.clone()).key("outline/card/detail").font(ROW_SMALL).color(c.text_muted).grow(1.0).min_w(0.0));
    if !m.time.is_empty() {
        second = second.child(text(m.time.clone()).key("outline/card/time").font(META).color(c.text_faint));
    }
    let body = col()
        .key("outline/card")
        .role("tooltip")
        .w(CARD_W)
        .gap(2.0)
        .pt(12.0)
        .pr(14.0)
        .pb(13.0)
        .pl(16.0)
        .child(text(m.headline.clone()).key("outline/card/title").font(k.ty.ui).color(c.text_base))
        .child(second);
    layer.push(popover::anchored(k, popover::raised(k, body, RADIUS_12), &format!("outline/tick/{}", m.id), Placement::Left, 10.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(text: &str, kind: &str, from: &str, pinned: &str) -> Todo {
        Todo { id: "T1".into(), text: text.into(), kind: kind.into(), from: from.into(), pinned: pinned.into(), ..Todo::default() }
    }

    #[test]
    fn a_to_do_becomes_a_tick() {
        let long = "x".repeat(150);
        let huge = "y".repeat(400);
        let cases = [
            // (to-do, headline, detail, time, width)
            (todo("CI failed on #329\nFlaky test; I reran it.", "do", "b-1", "2026-11-08T14:38:00-03:00"), "CI failed on #329", "Flaky test; I reran it.", "14:38", 9),
            (todo("\n  \nRenew the certificate", "do", "b-242", ""), "Renew the certificate", "to do · from b-242", "", 8),
            (todo("Heads up", "know", "", "2026-11-08T09:05:00Z"), "Heads up", "to know", "09:05", 8),
            (todo("Only", "", "b-9", "junk"), "Only", "from b-9", "", 8),
            (todo(&long, "do", "x", ""), long.as_str(), "to do · from x", "", 13),
            (todo(&huge, "do", "x", ""), huge.as_str(), "to do · from x", "", 14),
        ];
        for (t, headline, detail, time, width) in cases {
            let m = message(&t);
            assert_eq!((m.headline.as_str(), m.detail.as_str(), m.time.as_str(), m.width), (headline, detail, time, width), "{t:?}");
        }
    }

    #[test]
    fn new_ticks_are_the_ones_that_came_while_away() {
        let mut st = State::default();
        // First meeting: what is pinned is old, even when the tab is not shown.
        st.observe(1, &["T1", "T2"], false);
        assert!(!st.is_new(1, "T1") && !st.is_new(1, "T2"));
        // Away: T3 comes and stays new, again and again.
        st.observe(1, &["T1", "T2", "T3"], false);
        st.observe(1, &["T1", "T2", "T3"], false);
        assert!(st.is_new(1, "T3") && !st.is_new(1, "T2"));
        // Looking: all read. A tick that comes while looking is not new.
        st.observe(1, &["T1", "T2", "T3", "T4"], true);
        assert!(!st.is_new(1, "T3") && !st.is_new(1, "T4"));
        // Another tab is its own.
        assert!(!st.is_new(2, "T1"));
    }
}
