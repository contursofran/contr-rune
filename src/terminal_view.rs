//! The terminal panel: the panes (one or split), the reconnecting notice and the find bar.

use std::sync::atomic::Ordering;

use rune::{Color, Cursor, Drag, El, Icon, KeyEvent, Phase, Tx, WheelEvent, col, el, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, TextInput};

use crate::split::Split;
use crate::theme::*;
use crate::{App, Tab, find, grid, label, notch, outline, scroll_tab, select, shell, split};


pub(crate) fn terminal_panel(k: &Kit, s: &App) -> El {
    let panel = terminal_panel_body(k, s);
    let panel = match outline::ticks(k, s) {
        Some(o) => panel.child(o),
        None => panel,
    };
    match notch::notch(k, s) {
        Some(n) => panel.child(n),
        None => panel,
    }
}

fn terminal_panel_body(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let Some(t) = s.active_tab() else {
        return shell::terminal_panel(k).center().gap(8.0)
            .child(label("No terminal in this project", k.ty.ui, c.text_faint))
            .child(label("+ or Ctrl+Shift+T opens one", k.ty.text_xsmall, c.icon_faint));
    };
    let total = find_total(s, t);
    let panel = match &s.split {
        None => shell::terminal_panel(k).child(pane(k, s, 0, t.session.id(), false)),
        Some(sp) => shell::terminal_panel(k).child(split_row(k, s, sp)),
    };
    let panel = if s.find.open { panel.child(find_bar(k, s, total)) } else { panel };
    // An hc tab whose line is lost and attaching again: a quiet line at the bottom right.
    let detached = (0..2).filter_map(|i| s.pane_tab(i)).any(|id| s.tabs.iter().any(|t| t.session.id() == id && t.session.detached()));
    if detached { panel.child(reconnecting(k)) } else { panel }
}

/// The small "reconnecting" line over the terminal while an hc tab attaches again. Key `hc-banner`.
fn reconnecting(k: &Kit) -> El {
    text("reconnecting…").key("hc-banner").role("status").font(k.ty.text_small).color(k.c.text_faint).pin(None, Some(14.0), Some(10.0), None)
}

/// The number of find matches in the focused terminal.
pub(crate) fn find_total(s: &App, t: &Tab) -> usize {
    if !s.find.open { return 0; }
    let term = t.session.term().lock().unwrap();
    s.find.matches(t.session.id(), t.session.generation().load(Ordering::Acquire), &term).len()
}

/// Width of the divider's hit area; its hairline sits in the middle.
const GRIP: f32 = 7.0;

/// The two panes and the divider between them. Keys: `panes` (the row), `terminal/body` and
/// `terminal/body/1` (the panes), `split/divider`.
fn split_row(k: &Kit, s: &App, sp: &Split) -> El {
    let c = &k.c;
    let left = pane(k, s, 0, sp.panes[0], true).w_frac(sp.frac).min_w(0.0);
    // The divider straddles the panes' edge: a hairline inside the right pane, a wider grip over it.
    let hairline = el().key("split/hairline").w(1.0).fill(c.border_muted).pin(Some(0.0), Some(0.0), None, Some(0.0));
    let grip = el()
        .key("split/divider")
        .role("separator")
        .label("Resize panes")
        .w(GRIP)
        .cursor(Cursor::ResizeColumn)
        // As the kit's own resize grips: nearly invisible, but with a fill so it takes the pointer.
        .fill(Color::TRANSPARENT)
        .opacity(0.02)
        .pin(Some(-GRIP / 2.0), Some(0.0), None, Some(0.0))
        .focusable(false)
        .on_drag(|s: &mut App, d: &mut Drag, tx: &mut Tx| match d.phase {
            Phase::Down => d.data = s.split.as_ref().map_or(0.5, |sp| sp.frac),
            Phase::Move => {
                tx.instant();
                let frac = split::dragged_frac(d.data, d.dx, s.split_total);
                s.set_split_frac(frac);
            }
            Phase::Up => {}
        });
    let right = pane(k, s, 1, sp.panes[1], true).grow(1.0).min_w(0.0).child(hairline).child(grip);
    row()
        .key("panes")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_layout(|s: &mut App, [_, _, w, _]| s.split_total = w)
        .child(left)
        .child(right)
}

/// Pane `idx`, showing terminal `id`: the grid padded as the target (14 top, 20 sides, 12 bottom).
/// The first pane keeps the keys `terminal/body`, `grid` and `l<N>`; the second adds `/1` or `p1/`.
/// In a split the focused pane has an outline.
fn pane(k: &Kit, s: &App, idx: usize, id: u64, split: bool) -> El {
    let c = &k.c;
    let focused = !split || s.focus_idx() == idx;
    let Some(t) = s.tabs.iter().find(|t| t.session.id() == id) else { return col() };
    let (lines, selected) = {
        let term = t.session.term().lock().unwrap();
        let matches = if s.find.open && focused { s.find.matches(id, t.session.generation().load(Ordering::Acquire), &term) } else { Default::default() };
        let current = (!matches.is_empty()).then(|| s.find.current.min(matches.len() - 1));
        (grid::lines(&term, s.focused && focused, &grid::Ink::from_kit(k), &matches, current), select::text(&term))
    };
    let (grid_key, body_key, row_prefix) = if idx == 0 {
        ("grid".to_owned(), "terminal/body".to_owned(), String::new())
    } else {
        (format!("grid/{idx}"), format!("terminal/body/{idx}"), format!("p{idx}/"))
    };
    let mut grid_el = col()
        .key(grid_key)
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .clip_rounded(0.0)
        .on_layout(move |s: &mut App, [_, _, w, h]| s.set_grid_area(idx, w, h))
        // A press on the grid focuses its pane and gives the keys back to the shell (the find bar and its
        // highlights stay); a drag selects text, a double press selects a word.
        .on_drag(move |s: &mut App, d: &mut Drag, tx: &mut Tx| {
            if d.phase == Phase::Down {
                s.focus_pane(idx);
                if s.find.open {
                    s.find.edit.focused = false;
                    tx.blur();
                }
            }
            select::drag(s, idx, d);
        })
        .on_wheel(move |s: &mut App, ev: WheelEvent, _tx| {
            s.wheel += ev.dy;
            let lines = (s.wheel / LINE_H).trunc();
            if lines != 0.0 {
                s.wheel -= lines * LINE_H;
                if let Some(id) = s.pane_tab(idx) { scroll_tab(s, id, lines as i32); }
            }
            true
        });
    for (i, line) in lines.into_iter().enumerate() {
        grid_el = grid_el.child(
            text(line.text).key(format!("{row_prefix}l{i}")).font(TERMINAL).color(c.text_base).spans(line.spans).h(LINE_H),
        );
    }
    // The selected text, for the tree (and screen readers); it takes no room.
    if let Some(text) = selected {
        grid_el = grid_el.child(el().key(format!("terminal/selection{}", if idx == 0 { String::new() } else { format!("/{idx}") })).label(text).pin(Some(0.0), Some(0.0), None, None).w(0.0).h(0.0));
    }
    let body = col().key(body_key).min_h(0.0).pt(PANEL_PAD_TOP).px(PANEL_PAD_X).pb(PANEL_PAD_BOTTOM).child(grid_el);
    if !split { return body.grow(1.0).w_full(); }
    // The border is always there, so the grid keeps its size when the focus moves. The caller sets the width.
    body.h_full().border(1.0, if focused { k.alpha(c.border_focus, 0.5) } else { Color::TRANSPARENT })
}

/// The floating find bar at the top right of the terminal card: the input, "3 of 12", previous,
/// next and close. Keys: `find`, `find/input`, `find/count`, `find/prev`, `find/next`, `find/close`.
pub(crate) fn find_bar(k: &Kit, s: &App, total: usize) -> El {
    let c = &k.c;
    let count = find::count_label(s.find.query(), s.find.current, total);
    let input = TextInput::new(&s.find.edit)
        .placeholder("Find")
        .label("Find in terminal")
        .width(Some(180.0))
        .on_edit(|s: &mut App| &mut s.find.edit)
        .on_change(|s: &mut App, _tx| s.find_retarget())
        .on_key(|s: &mut App, ev: &KeyEvent, tx| s.find_key(ev, tx))
        .build(k)
        .key("find/input");
    let button = |icon: &'static Icon, label: &'static str, enabled: bool| {
        IconButton::new(icon)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label)
            .help(label)
            .disabled(!enabled)
    };
    let mut bar = row()
        .key("find")
        .role("search")
        .pin(None, Some(10.0), Some(12.0), None)
        .px(4.0)
        .pt(4.0)
        .pb(4.0)
        .gap(4.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(input)
        .child(text(count).key("find/count").font(k.ty.text_small).color(c.text_muted).w(64.0).px(4.0))
        .child(button(&rune_kit::glyphs::CHEVRON_UP, "Previous match", total > 0).on_press(|s: &mut App| s.find_step(false)).build(k).key("find/prev"))
        .child(button(&rune_kit::glyphs::CHEVRON_DOWN, "Next match", total > 0).on_press(|s: &mut App| s.find_step(true)).build(k).key("find/next"))
        .child(
            button(&rune_kit::glyphs::CLOSE, "Close search", true)
                .on_press_tx(|s: &mut App, tx: &mut Tx| { s.find.close(); tx.blur(); })
                .build(k)
                .key("find/close"),
        );
    for shadow in k.shadow_layers(&k.elevation.overlay) {
        bar = bar.box_shadow(shadow);
    }
    bar
}
