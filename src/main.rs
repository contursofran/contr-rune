//! contr-rune: contr's sidebar, terminal tabs and terminal, rebuilt on Rune.
//!
//! Left: projects, each with its terminals, then the terminals grouped by state (contr's
//! "Agents" section). Top: the tabs of the selected project and `+`. Middle: the selected
//! terminal, a real shell in a PTY.
//!
//! `CONTR_PROJECTS=~/a:~/b cargo run` opens those folders as projects (default: the current
//! folder and home). Shortcuts: Ctrl+Shift+T new tab, Ctrl+Shift+W close, Ctrl+Tab /
//! Ctrl+Shift+Tab or Ctrl+PageDown / PageUp switch, Ctrl+Shift+V paste (⌘ on macOS).

mod grid;
mod keys;
mod session;
mod shell;
mod theme;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use alacritty_terminal::term::TermMode;
use rune_kit::shell::{TabDragState, TabItem, TabItemStatus, TabOrientation, TabOverflow, TabReorder, TabStrip};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit};
use rune::{AppSpec, Color, Cx, El, Icon, Key, KeyEvent, Tx, Waker, WheelEvent, col, el, row, spacer, text};

use session::{Session, Size, now_ms};
use theme::*;

/// Output within this long ago makes a terminal "working".
const WORKING_MS: u64 = 1_500;

struct Project {
    name: String,
    path: PathBuf,
    /// The tab to show when the project is selected again.
    last_tab: Option<u64>,
}

struct Tab {
    project: usize,
    session: Session,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Working,
    Idle,
}

struct App {
    projects: Vec<Project>,
    tabs: Vec<Tab>,
    selected_project: usize,
    active: Option<u64>,
    waker: Option<Waker>,
    next_id: u64,
    /// Terminal size from the laid-out grid area.
    size: Size,
    wheel: f32,
    statuses: Vec<(u64, Status)>,
    focused: bool,
    error: Option<String>,
    /// Drag state of the title bar's tab strip.
    tab_drag: TabDragState,
    /// The one kit every view reads colours, type and radius from.
    kit: Kit,
}

fn init() -> App {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let paths: Vec<PathBuf> = match std::env::var("CONTR_PROJECTS") {
        Ok(list) => list
            .split(':')
            .filter(|p| !p.is_empty())
            .map(|p| match p.strip_prefix("~/") { Some(rest) => home.join(rest), None => PathBuf::from(p) })
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
            if cwd == home { vec![home] } else { vec![cwd, home] }
        }
    };
    let projects = paths
        .into_iter()
        .map(|path| Project {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()),
            path,
            last_tab: None,
        })
        .collect();
    App {
        projects,
        tabs: Vec::new(),
        selected_project: 0,
        active: None,
        waker: None,
        next_id: 1,
        size: Size { cols: 100, rows: 30 },
        wheel: 0.0,
        statuses: Vec::new(),
        focused: true,
        error: None,
        tab_drag: TabDragState::new(),
        kit: Kit::neutral(true),
    }
}

impl App {
    fn open_tab(&mut self, project: usize) {
        let Some(p) = self.projects.get(project) else { return };
        let id = self.next_id;
        match Session::spawn(id, &p.path, self.size, self.waker.clone()) {
            Ok(session) => {
                self.next_id += 1;
                self.tabs.push(Tab { project, session });
                self.select_tab(id);
            }
            Err(e) => self.error = Some(format!("couldn't start a shell: {e}")),
        }
    }

    fn select_tab(&mut self, id: u64) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.session.id == id) else { return };
        t.session.resize(self.size);
        let project = t.project;
        self.selected_project = project;
        self.projects[project].last_tab = Some(id);
        self.active = Some(id);
    }

    fn select_project(&mut self, project: usize) {
        self.selected_project = project;
        let last = self.projects[project].last_tab.filter(|id| self.tabs.iter().any(|t| t.session.id == *id));
        match last.or_else(|| self.project_tabs(project).next().map(|t| t.session.id)) {
            Some(id) => self.select_tab(id),
            None => self.active = None,
        }
    }

    fn close_tab(&mut self, id: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.session.id == id) else { return };
        let project = self.tabs[i].project;
        // Dropping the session closes the PTY; the shell gets SIGHUP.
        let siblings: Vec<u64> = self.project_tabs(project).map(|t| t.session.id).collect();
        let pos = siblings.iter().position(|s| *s == id).unwrap_or(0);
        self.tabs.remove(i);
        if self.active == Some(id) {
            let rest: Vec<u64> = self.project_tabs(project).map(|t| t.session.id).collect();
            match rest.get(pos.min(rest.len().saturating_sub(1))) {
                Some(next) => self.select_tab(*next),
                None => self.active = None,
            }
        }
    }

    /// Move the `from`-th tab of `project` to position `to` among that project's tabs.
    fn reorder_tab(&mut self, project: usize, from: usize, to: usize) {
        let slots: Vec<usize> = self.tabs.iter().enumerate().filter(|(_, t)| t.project == project).map(|(i, _)| i).collect();
        if from >= slots.len() || to >= slots.len() || from == to { return; }
        // The project's tabs keep their slots among the others; only their order changes.
        let mut moved: Vec<Tab> = slots.iter().rev().map(|i| self.tabs.remove(*i)).collect();
        moved.reverse();
        let t = moved.remove(from);
        moved.insert(to, t);
        for (slot, t) in slots.iter().zip(moved) {
            self.tabs.insert(*slot, t);
        }
    }

    fn cycle_tab(&mut self, step: i32) {
        let ids: Vec<u64> = self.project_tabs(self.selected_project).map(|t| t.session.id).collect();
        if ids.is_empty() { return; }
        let cur = self.active.and_then(|a| ids.iter().position(|i| *i == a)).unwrap_or(0) as i32;
        let next = (cur + step).rem_euclid(ids.len() as i32) as usize;
        self.select_tab(ids[next]);
    }

    fn project_tabs(&self, project: usize) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(move |t| t.project == project)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tabs.iter().find(|t| t.session.id == id))
    }

    fn status(&self, id: u64) -> Status {
        self.statuses.iter().find(|(i, _)| *i == id).map(|(_, s)| *s).unwrap_or(Status::Idle)
    }

    fn set_grid_area(&mut self, w: f32, h: f32) {
        let size = Size {
            cols: ((w / CELL_W).floor() as u16).max(2),
            rows: ((h / LINE_H).floor() as u16).max(1),
        };
        if size == self.size { return; }
        self.size = size;
        if let Some(id) = self.active
            && let Some(t) = self.tabs.iter_mut().find(|t| t.session.id == id)
        {
            t.session.resize(size);
        }
    }

    fn paste(&mut self, text: &str) {
        let Some(t) = self.active_tab() else { return };
        let bracketed = t.session.term.lock().unwrap().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            t.session.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            t.session.write(text.as_bytes());
        }
    }
}

// ───────────────────────────── hooks ─────────────────────────────

fn setup(s: &mut App, waker: Waker) {
    s.waker = Some(waker);
    if s.tabs.is_empty() && !s.projects.is_empty() {
        s.open_tab(0);
    }
}

fn poll(s: &mut App, _t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    // Shells that exited (`exit`, Ctrl+D) close their tab, as in contr.
    let exited: Vec<u64> = s.tabs.iter().filter(|t| t.session.exited.load(Ordering::Acquire)).map(|t| t.session.id).collect();
    for id in exited {
        s.close_tab(id);
        changed = true;
    }
    for t in &s.tabs {
        if t.session.dirty.swap(false, Ordering::AcqRel) { changed = true; }
    }
    let now = now_ms();
    let statuses: Vec<(u64, Status)> = s
        .tabs
        .iter()
        .map(|t| {
            let quiet = now.saturating_sub(t.session.last_output.load(Ordering::Relaxed));
            (t.session.id, if quiet < WORKING_MS { Status::Working } else { Status::Idle })
        })
        .collect();
    if statuses.iter().any(|(_, st)| *st == Status::Working) {
        // Look again when the newest output turns quiet, to drop the working dot.
        tx.wake_after(Duration::from_millis(500));
    }
    if statuses != s.statuses {
        s.statuses = statuses;
        changed = true;
    }
    changed
}

/// The window's shortcut chord: ⌘ on macOS, Ctrl+Shift elsewhere (plain Ctrl belongs to the shell).
fn is_shortcut(ev: &KeyEvent, c: char) -> bool {
    let m = ev.mods;
    let chord = if cfg!(target_os = "macos") { m.cmd && !m.ctrl } else { m.ctrl && m.shift };
    chord && ev.is_command(c)
}

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if is_shortcut(ev, 't') {
        s.open_tab(s.selected_project);
        return true;
    }
    if is_shortcut(ev, 'w') {
        if let Some(id) = s.active { s.close_tab(id); }
        return true;
    }
    if is_shortcut(ev, 'v') {
        tx.read_clipboard(|s: &mut App, content, _tx| s.paste(content.text()));
        return true;
    }
    match ev.key {
        Key::Tab if ev.mods.ctrl => { s.cycle_tab(if ev.mods.shift { -1 } else { 1 }); return true; }
        Key::PageDown if ev.mods.ctrl => { s.cycle_tab(1); return true; }
        Key::PageUp if ev.mods.ctrl => { s.cycle_tab(-1); return true; }
        Key::PageUp if ev.mods.shift => { scroll_active(s, s.size.rows as i32 - 1); return true; }
        Key::PageDown if ev.mods.shift => { scroll_active(s, -(s.size.rows as i32 - 1)); return true; }
        _ => {}
    }
    let Some(t) = s.active_tab() else { return false };
    let mode = *t.session.term.lock().unwrap().mode();
    match keys::bytes(ev, mode) {
        Some(b) => { t.session.write(&b); true }
        None => false,
    }
}

fn scroll_active(s: &mut App, lines: i32) {
    if let Some(t) = s.active_tab() { t.session.scroll(lines); }
}

// ───────────────────────────── view ─────────────────────────────

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = &s.kit;
    let mut layer = Vec::new();
    let bar = title_bar(k, s, cx, &mut layer);
    shell::window(k, bar, sidebar(k, s), terminal_panel(k, s), shell::files_panel(k), layer)
        .on_window_focus(|s: &mut App, focused, _tx| s.focused = focused)
}

fn label(t: impl Into<String>, style: rune::tok::TextStyle, color: Color) -> El {
    text(t.into()).font(style).color(color)
}

fn hover_tint(k: &Kit) -> Color { k.c.hover }

// Sidebar ─────────────

fn sidebar(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let mut projects = col().gap(2.0).px(6.0);
    for (i, p) in s.projects.iter().enumerate() {
        projects = projects.child(project_row(k, s, i, p));
        for t in s.project_tabs(i) {
            projects = projects.child(terminal_row(k, s, t, 22.0));
        }
    }

    let working: Vec<&Tab> = s.tabs.iter().filter(|t| s.status(t.session.id) == Status::Working).collect();
    let idle: Vec<&Tab> = s.tabs.iter().filter(|t| s.status(t.session.id) == Status::Idle).collect();
    let mut agents = col().gap(2.0).px(6.0);
    for (title, color, group) in [("Working", c.text_base, &working), ("Idle", c.text_faint, &idle)] {
        if group.is_empty() { continue; }
        agents = agents.child(label(format!("{title} {}", group.len()), k.ty.text_xsmall, color).px(6.0).pt(6.0).pb(2.0));
        for t in group.iter() {
            agents = agents.child(terminal_row(k, s, t, 0.0));
        }
    }

    shell::sidebar_panel(k)
        .child(
            row().h(40.0).px(14.0).items_center().window_drag()
                .child(label("contr", k.ty.text_xsmall_medium, c.text_base))
                .child(spacer())
                .child(label("rune", k.ty.text_xsmall, c.icon_faint)),
        )
        .child(section_header(k, "Spaces", s.projects.len()))
        .child(label("This computer", k.ty.text_xsmall, c.text_faint).px(14.0).pb(4.0))
        .child(projects)
        .child(spacer())
        .child(section_header(k, "Terminals", s.tabs.len()))
        .child(agents)
        .child(
            row().h(30.0).px(14.0).items_center()
                .child(label(s.error.clone().unwrap_or_default(), k.ty.text_xsmall, c.danger.icon).truncate()),
        )
}

fn section_header(k: &Kit, title: &str, n: usize) -> El {
    let c = &k.c;
    row().px(14.0).pt(10.0).pb(6.0).items_center()
        .child(label(title, k.ty.ui, c.text_faint))
        .child(spacer())
        .child(label(n.to_string(), k.ty.text_xsmall, c.icon_faint))
}

fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
    let s: String = match parts.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    s.to_uppercase()
}

fn project_row(k: &Kit, s: &App, i: usize, p: &Project) -> El {
    let c = &k.c;
    let selected = i == s.selected_project;
    let count = s.project_tabs(i).count();
    let tile = el().size(20.0, 20.0).radius(k.radius.sm).fill(c.pressed).border(1.0, c.border_muted).center()
        .child(label(initials(&p.name), k.ty.text_xsmall, c.text_muted));
    let add = el().key(format!("add-{i}")).size(20.0, 20.0).radius(k.radius.sm).center().hover(c.pressed)
        .role("button").label("New terminal here")
        .on_press(move |s: &mut App| s.open_tab(i))
        .child(label("+", k.ty.ui, c.text_faint));
    let mut r = row().key(format!("project-{i}")).px(8.0).py(6.0).radius(k.radius.md).gap(8.0).items_center()
        .role("button").label(p.name.clone())
        .hover(hover_tint(k))
        .on_press(move |s: &mut App| s.select_project(i))
        .child(tile)
        .child(label(p.name.clone(), k.ty.ui, if selected { c.text_base } else { c.text_muted }).truncate().shrink().min_w(0.0))
        .child(spacer())
        .child(label(if count > 0 { count.to_string() } else { String::new() }, k.ty.text_xsmall, c.icon_faint))
        .child(add);
    if selected { r = r.fill(c.pressed).border(1.0, c.border_muted); }
    r
}

fn status_dot(k: &Kit, st: Status) -> El {
    let c = &k.c;
    match st {
        Status::Working => el().size(7.0, 7.0).radius(3.5).fill(c.text_base),
        Status::Idle => el().size(9.0, 9.0).radius(4.5).border(1.6, c.icon_faint),
    }
}

fn terminal_row(k: &Kit, s: &App, t: &Tab, indent: f32) -> El {
    let c = &k.c;
    let id = t.session.id;
    let active = s.active == Some(id);
    let st = s.status(id);
    let second = match st {
        Status::Working => "working".to_string(),
        Status::Idle => {
            let age = now_ms().saturating_sub(t.session.last_output.load(Ordering::Relaxed)) / 1000;
            format!("idle · {}", fmt_age(age))
        }
    };
    let mut r = col().key(format!("term-{id}-{indent}")).px(8.0).py(5.0).ml(indent).radius(k.radius.md).gap(3.0)
        .role("button").label(t.session.label())
        .hover(hover_tint(k))
        .on_press(move |s: &mut App| s.select_tab(id))
        .child(
            row().gap(8.0).items_center()
                .child(label(">_", k.ty.text_xsmall, c.text_faint))
                .child(label(t.session.label(), k.ty.ui, if active { c.text_base } else { c.text_muted }).truncate().shrink().min_w(0.0))
                .child(spacer())
                .child(status_dot(k, st)),
        )
        .child(label(second, k.ty.text_xsmall, c.text_faint).pl(26.0));
    if active { r = r.fill(c.pressed); }
    r
}

/// contr's ages: 5s, 3m, 2h. They move on a repaint, never on a timer of their own.
fn fmt_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

// Tabs and terminal ─────────────

// Icons from main.html (16 point view box), as in `examples/kit_preview.rs`.
static TERM: Icon = Icon {
    name: "contr-term",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M3 4.5l3 3.5-3 3.5M8 12h5"/></g>"#,
};
static PLUS: Icon = Icon {
    name: "contr-plus",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3v10M3 8h10"/></g>"#,
};

fn tab_id(id: u64) -> String { id.to_string() }

fn title_bar(k: &Kit, s: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> El {
    let project = s.selected_project;
    let titles: Vec<(u64, String)> = s.project_tabs(project).map(|t| (t.session.id, t.session.tab_title())).collect();
    let items = titles.iter().map(|(id, title)| {
        let status = (s.status(*id) == Status::Working).then_some(TabItemStatus::Busy);
        TabItem::new(tab_id(*id), title.as_str()).icon(&TERM).status(status)
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
        .on_close(|s: &mut App, id: &str, _tx| { if let Ok(id) = id.parse() { s.close_tab(id); } })
        .on_reorder(move |s: &mut App, r: TabReorder, _tx| s.reorder_tab(project, r.from, r.to))
        .build_layered(k, layer);
    // The leading area stays free for the macOS traffic lights; the tabs start after it. K5 fills the right side.
    shell::titlebar(k, shell::lights_gap(), strip, Vec::new())
}

fn terminal_panel(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let Some(t) = s.active_tab() else {
        return shell::terminal_panel(k).center().gap(8.0)
            .child(label("No terminal in this project", k.ty.ui, c.text_faint))
            .child(label("+ or Ctrl+Shift+T opens one", k.ty.text_xsmall, c.icon_faint));
    };
    let cwd = t.session.cwd.display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = if !home.is_empty() && cwd.starts_with(&home) { format!("~{}", &cwd[home.len()..]) } else { cwd };
    let header = row().h(34.0).px(12.0).gap(8.0).items_center()
        .child(label(">_", k.ty.text_xsmall, c.text_faint))
        .child(label(t.session.label(), k.ty.ui, c.text_base).truncate().shrink().min_w(0.0))
        .child(label("·", k.ty.ui, c.icon_faint))
        .child(label(cwd, k.ty.text_xsmall, c.text_faint).truncate().shrink().min_w(0.0))
        .child(spacer())
        .child(label(format!("{}×{}", s.size.cols, s.size.rows), k.ty.text_xsmall, c.icon_faint));

    let lines = {
        let term = t.session.term.lock().unwrap();
        grid::lines(&term, s.focused, &grid::Ink::from_kit(k))
    };
    let mut grid_el = col()
        .key("grid")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .clip_rounded(0.0)
        .on_layout(|s: &mut App, [_, _, w, h]| s.set_grid_area(w, h))
        .on_wheel(|s: &mut App, ev: WheelEvent, _tx| {
            s.wheel += ev.dy;
            let lines = (s.wheel / LINE_H).trunc();
            if lines != 0.0 {
                s.wheel -= lines * LINE_H;
                scroll_active(s, lines as i32);
            }
            true
        });
    for (i, line) in lines.into_iter().enumerate() {
        grid_el = grid_el.child(
            text(line.text).key(format!("l{i}")).font(TERMINAL).color(c.text_base).spans(line.spans).h(LINE_H),
        );
    }

    shell::terminal_panel(k)
        .child(header)
        .child(el().w_full().h(1.0).fill(c.border_muted))
        .child(col().grow(1.0).min_h(0.0).w_full().px(10.0).py(8.0).child(grid_el))
}

fn main() {
    let spec = AppSpec::<App> {
        min_size: Some((720.0, 420.0)),
        fonts: FONTS,
        families: FAMILIES,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: &[("three-tabs", |s| {
            s.open_tab(0);
            s.open_tab(0);
            s.open_tab(0);
        })],
        ..AppSpec::new("contr", (1280.0, 800.0), init, view)
    };
    // The 38 px title bar is ours; the traffic lights sit in its left area (first circle at x 18, centred).
    rune::App::new(spec).titlebar(rune::Titlebar::new(TITLEBAR_HEIGHT).traffic_lights(17.0, 12.0)).run();
}
