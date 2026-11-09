//! The title bar: the tab strip and its trailing buttons (diff summary, split, search, files toggle).

use rune::{Cx, El, Icon, PointerButton, Tx};
use rune_kit::shell::{TabItem, TabItemStatus, TabOrientation, TabOverflow, TabReorder, TabStrip};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, MenuItem, State, context_menu};

use crate::theme::*;
use crate::{App, PLUS, Status, TERM, shell, sidebar};


static SPLIT: Icon = Icon {
    name: "contr-split",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M8 2.5v11"/></g>"#,
};
static SEARCH: Icon = Icon {
    name: "contr-search",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5l3.5 3.5"/></g>"#,
};
static PANEL_RIGHT: Icon = Icon {
    name: "contr-panel-right",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M10 2.5v11"/></g>"#,
};

pub(crate) fn tab_id(id: u64) -> String { id.to_string() }

pub(crate) fn title_bar(k: &Kit, s: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> El {
    let project = s.selected_project;
    let titles: Vec<(u64, String)> = s.project_tabs(project).map(|t| (t.session.id(), t.session.tab_title())).collect();
    let items = titles.iter().map(|(id, title)| {
        // Needs-you wins over busy.
        let status = if s.tab_needs_you(*id) {
            Some(TabItemStatus::Attention)
        } else {
            (s.status(*id) == Status::Working).then_some(TabItemStatus::Busy)
        };
        let tab_id_n = *id;
        let item = match s.tab_agent(*id) {
            // An agent's tab: its title and state glyph (H6) replace the terminal title and prompt icon.
            Some(a) => TabItem::new(tab_id(*id), a.display_title().to_owned()).avatar(sidebar::agent_glyph(k, a, a.orchestrates(&s.agents))),
            None => TabItem::new(tab_id(*id), title.as_str()).icon(&TERM).status(status),
        };
        // A secondary press opens the tab's menu (T3).
        item.marked(s.tab_menu.is_some_and(|(open, _)| open == tab_id_n))
            .decorate(move |tab, _| tab.on_secondary_press(move |s: &mut App, at, _tx| s.tab_menu = Some((tab_id_n, at))))
    });
    let active = s.active.map(tab_id);
    let plus = IconButton::new(&PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("New tab")
        .on_press(move |s: &mut App| s.open_tab(project))
        .build(k)
        .key("new-tab");
    let strip = TabStrip::new("top", TabOrientation::Horizontal)
        .tabs(items)
        .active(active.as_deref())
        .overflow(TabOverflow::ShrinkThenScroll { min_width: 28.0, max_width: TAB_MAX_WIDTH, pinned_width: 36.0, fade_width: 24.0 })
        .viewport_width(cx.layout_size("top").map(|z| z[0]))
        .drag_state(&s.tab_drag, |s: &mut App| &mut s.tab_drag)
        .close_icon(k.glyph.close)
        .trailing_action(plus)
        .on_select(|s: &mut App, id: &str, _tx| { if let Ok(id) = id.parse() { s.select_tab(id); } })
        .on_close(|s: &mut App, id: &str, tx| {
            let Ok(id) = id.parse() else { return };
            // The kit closes on the button and on a middle click alike. A middle click leaves an
            // agent's tab alone: only its button or menu detaches it (T3).
            if tx.button() == PointerButton::Middle && s.tab_agent(id).is_some() {
                return;
            }
            s.close_tab(id);
        })
        .on_reorder(move |s: &mut App, r: TabReorder, _tx| s.reorder_tab(project, r.from, r.to))
        .build_layered(k, layer);
    tab_menu(k, s, layer);
    // The leading area stays free for the macOS traffic lights; the tabs start after it.
    shell::titlebar(k, shell::lights_gap(), strip, trailing(k, s))
}

/// The title bar's right side: diff summary, split, search, files toggle.
fn trailing(k: &Kit, s: &App) -> Vec<El> {
    let mut items = Vec::new();
    if s.update.ready.is_some() {
        items.push(
            Button::new("Restart to update")
                .variant(ButtonVariant::Neutral)
                .control_size(ControlSize::Small)
                .help("An update is ready")
                .on_press(|s: &mut App| s.update.panel = !s.update.panel)
                .build(k)
                .key("update-pill"),
        );
    }
    if let Some(text) = s.diff.and_then(|d| d.text()) {
        items.push(
            Button::new(text)
                .variant(ButtonVariant::Ghost)
                .help("Show changed files")
                .on_press(|s: &mut App| s.show_files = true)
                .build(k)
                .key("diff-summary"),
        );
    }
    let icon = |i: &'static Icon, label: &str| IconButton::new(i).variant(IconButtonVariant::GhostMuted).control_size(ControlSize::Large).label(label.to_owned());
    let split_chord = if cfg!(target_os = "macos") { "cmd+d" } else { "ctrl+shift+d" };
    items.push(
        icon(&SPLIT, "Split terminal")
            .help("Split terminal")
            .shortcut(split_chord)
            .state(if s.split.is_some() { State::Pressed } else { State::Rest })
            // The button gives the focus back: a space typed next would press it again.
            .on_press_tx(|s: &mut App, tx: &mut Tx| { s.toggle_split(); tx.blur(); })
            .build(k)
            .key("split"),
    );
    let find_chord = if cfg!(target_os = "macos") { "cmd+f" } else { "ctrl+shift+f" };
    items.push(
        icon(&SEARCH, "Search")
            .help("Search")
            .shortcut(find_chord)
            .state(if s.find.open { State::Pressed } else { State::Rest })
            .on_press(|s: &mut App| s.open_find())
            .build(k)
            .key("search"),
    );
    let label = if s.show_files { "Hide files" } else { "Show files" };
    let chord = if cfg!(target_os = "macos") { "cmd+b" } else { "ctrl+shift+b" };
    items.push(
        icon(&PANEL_RIGHT, label)
            .help(label)
            .shortcut(chord)
            .state(if s.show_files { State::Pressed } else { State::Rest })
            .on_press(|s: &mut App| s.toggle_files())
            .build(k)
            .key("files-toggle"),
    );
    items
}

/// The open tab menu (T3), mounted in `layer`: Close (off on an agent's tab, which only detaches
/// through its button) and Close others.
fn tab_menu(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let Some((id, at)) = s.tab_menu else { return };
    let Some(i) = s.project_tabs(s.selected_project).position(|t| t.session.id() == id) else { return };
    let agent = s.tab_agent(id).is_some();
    context_menu(format!("tab/{i}/menu"), rune::el(), Some(at))
        .item(MenuItem::new("Close").disabled(agent).on_select(move |s: &mut App| {
            s.tab_menu = None;
            s.close_tab(id);
        }))
        .item(MenuItem::new("Close others").on_select(move |s: &mut App| {
            s.tab_menu = None;
            s.close_other_tabs(id);
        }))
        .on_close(|s: &mut App| s.tab_menu = None)
        .build(k, layer);
}
