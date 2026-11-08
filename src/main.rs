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
mod theme;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use alacritty_terminal::term::TermMode;
use rune::{AppSpec, Color, Cx, El, Key, KeyEvent, Tx, Waker, WheelEvent, col, el, row, spacer, text};

use session::{Session, Size, now_ms};
use theme::*;

const SIDEBAR_W: f32 = 260.0;
/// Output within this long ago makes a terminal "working" (amber dot).
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
        // Look again when the newest output turns quiet, to drop the amber dot.
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

fn view(s: &App, _cx: &mut Cx<App>) -> El {
    row()
        .w_full()
        .h_full()
        .fill(BG_WINDOW())
        .on_window_focus(|s: &mut App, focused, _tx| s.focused = focused)
        .child(sidebar(s))
        .child(el().w(1.0).h_full().fill(LINE()))
        .child(main_area(s))
}

fn label(t: impl Into<String>, style: rune::tok::TextStyle, color: Color) -> El {
    text(t.into()).font(style).color(color)
}

fn hover_tint() -> Color { Color::WHITE.alpha(0.035) }

// Sidebar ─────────────

fn sidebar(s: &App) -> El {
    let mut projects = col().gap(2.0).px(6.0);
    for (i, p) in s.projects.iter().enumerate() {
        projects = projects.child(project_row(s, i, p));
        for t in s.project_tabs(i) {
            projects = projects.child(terminal_row(s, t, 22.0));
        }
    }

    let working: Vec<&Tab> = s.tabs.iter().filter(|t| s.status(t.session.id) == Status::Working).collect();
    let idle: Vec<&Tab> = s.tabs.iter().filter(|t| s.status(t.session.id) == Status::Idle).collect();
    let mut agents = col().gap(2.0).px(6.0);
    for (title, color, group) in [("Working", ATTENTION(), &working), ("Idle", TEXT_DIM(), &idle)] {
        if group.is_empty() { continue; }
        agents = agents.child(label(format!("{title} {}", group.len()), UI_SMALL, color).px(6.0).pt(6.0).pb(2.0));
        for t in group.iter() {
            agents = agents.child(terminal_row(s, t, 0.0));
        }
    }

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .min_h(0.0)
        .fill(BG_BARS())
        .child(
            row().h(40.0).px(14.0).items_center().window_drag()
                .child(label("contr", UI_BOLD, TEXT()))
                .child(spacer())
                .child(label("rune", UI_SMALL, TEXT_FAINT())),
        )
        .child(section_header("Spaces", s.projects.len()))
        .child(label("This computer", UI_SMALL, TEXT_DIM()).px(14.0).pb(4.0))
        .child(projects)
        .child(spacer())
        .child(section_header("Terminals", s.tabs.len()))
        .child(agents)
        .child(
            row().h(30.0).px(14.0).items_center()
                .child(label(s.error.clone().unwrap_or_default(), UI_SMALL, FAILED()).truncate()),
        )
}

fn section_header(title: &str, n: usize) -> El {
    row().px(14.0).pt(10.0).pb(6.0).items_center()
        .child(label(title, UI, TEXT_DIM()))
        .child(spacer())
        .child(label(n.to_string(), UI_SMALL, TEXT_FAINT()))
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

fn project_row(s: &App, i: usize, p: &Project) -> El {
    let selected = i == s.selected_project;
    let count = s.project_tabs(i).count();
    let tile = el().size(20.0, 20.0).radius(5.0).fill(BG_CARD()).border(1.0, CARD_EDGE()).center()
        .child(label(initials(&p.name), UI_SMALL, TEXT_2()));
    let add = el().key(format!("add-{i}")).size(20.0, 20.0).radius(5.0).center().hover(Color::WHITE.alpha(0.08))
        .role("button").label("New terminal here")
        .on_press(move |s: &mut App| s.open_tab(i))
        .child(label("+", UI, TEXT_DIM()));
    let mut r = row().key(format!("project-{i}")).px(8.0).py(6.0).radius(6.0).gap(8.0).items_center()
        .role("button").label(p.name.clone())
        .hover(hover_tint())
        .on_press(move |s: &mut App| s.select_project(i))
        .child(tile)
        .child(label(p.name.clone(), UI, if selected { TEXT() } else { TEXT_2() }).truncate().shrink().min_w(0.0))
        .child(spacer())
        .child(label(if count > 0 { count.to_string() } else { String::new() }, UI_SMALL, TEXT_FAINT()))
        .child(add);
    if selected { r = r.fill(BG_CARD()).border(1.0, CARD_EDGE()); }
    r
}

fn status_dot(st: Status) -> El {
    match st {
        Status::Working => el().size(7.0, 7.0).radius(3.5).fill(ATTENTION()),
        Status::Idle => el().size(9.0, 9.0).radius(4.5).border(1.6, srgb(0x6A6A6A)),
    }
}

fn terminal_row(s: &App, t: &Tab, indent: f32) -> El {
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
    let mut r = col().key(format!("term-{id}-{indent}")).px(8.0).py(5.0).ml(indent).radius(6.0).gap(3.0)
        .role("button").label(t.session.label())
        .hover(hover_tint())
        .on_press(move |s: &mut App| s.select_tab(id))
        .child(
            row().gap(8.0).items_center()
                .child(label(">_", UI_SMALL, TEXT_DIM()))
                .child(label(t.session.label(), UI, if active { TEXT() } else { TEXT_2() }).truncate().shrink().min_w(0.0))
                .child(spacer())
                .child(status_dot(st)),
        )
        .child(label(second, UI_SMALL, TEXT_DIM()).pl(26.0));
    if active { r = r.fill(BG_CARD()); }
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

fn main_area(s: &App) -> El {
    col().grow(1.0).h_full().min_w(0.0).min_h(0.0)
        .child(top_bar(s))
        .child(el().w_full().h(1.0).fill(LINE()))
        .child(
            col().grow(1.0).min_h(0.0).w_full().pt(8.0).pl(8.0).pr(8.0).pb(8.0)
                .child(terminal_card(s)),
        )
}

fn top_bar(s: &App) -> El {
    let mut tabs = row().gap(4.0).items_center().min_w(0.0).shrink();
    for t in s.project_tabs(s.selected_project) {
        let id = t.session.id;
        let selected = s.active == Some(id);
        let close = el().key(format!("close-{id}")).size(16.0, 16.0).radius(4.0).center()
            .hover(Color::WHITE.alpha(0.1))
            .role("button").label("Close tab")
            .on_press(move |s: &mut App| s.close_tab(id))
            .child(label("×", UI, if selected { TEXT_2() } else { TEXT_FAINT() }));
        let mut tab = row().key(format!("tab-{id}")).h(28.0).pl(10.0).pr(6.0).radius(7.0).gap(7.0).items_center()
            .max_w(220.0).min_w(0.0)
            .role("tab").label(t.session.label()).checked(selected)
            .hover(hover_tint())
            .on_press(move |s: &mut App| s.select_tab(id))
            .child(label(">_", UI_SMALL, TEXT_DIM()))
            .child(label(t.session.label(), UI, if selected { TEXT() } else { TEXT_2() }).truncate().shrink().min_w(0.0));
        if s.status(id) == Status::Working {
            tab = tab.child(el().size(7.0, 7.0).radius(3.5).fill(ATTENTION()));
        }
        tab = tab.child(close);
        if selected { tab = tab.fill(BG_CARD()).border(1.0, CARD_EDGE()); }
        tabs = tabs.child(tab);
    }
    let project = s.selected_project;
    let plus = el().key("new-tab").size(28.0, 28.0).radius(6.0).center()
        .hover(Color::WHITE.alpha(0.07))
        .role("button").label("New terminal")
        .on_press(move |s: &mut App| s.open_tab(project))
        .child(label("+", UI_BOLD, TEXT_DIM()));
    let name = s.projects.get(project).map(|p| p.name.clone()).unwrap_or_default();
    row().h(40.0).w_full().px(8.0).gap(6.0).items_center().fill(BG_BARS()).window_drag()
        .child(tabs)
        .child(plus)
        .child(spacer())
        .child(label(name, UI_SMALL, TEXT_DIM()).pr(6.0))
}

fn terminal_card(s: &App) -> El {
    let Some(t) = s.active_tab() else {
        return col().w_full().h_full().radius(10.0).fill(TERM_BG()).border(1.0, LINE()).center().gap(8.0)
            .child(label("No terminal in this project", UI, TEXT_DIM()))
            .child(label("+ or Ctrl+Shift+T opens one", UI_SMALL, TEXT_FAINT()));
    };
    let cwd = t.session.cwd.display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = if !home.is_empty() && cwd.starts_with(&home) { format!("~{}", &cwd[home.len()..]) } else { cwd };
    let header = row().h(34.0).px(12.0).gap(8.0).items_center()
        .child(label(">_", UI_SMALL, TEXT_DIM()))
        .child(label(t.session.label(), UI, TEXT()).truncate().shrink().min_w(0.0))
        .child(label("·", UI, TEXT_FAINT()))
        .child(label(cwd, UI_SMALL, TEXT_DIM()).truncate().shrink().min_w(0.0))
        .child(spacer())
        .child(label(format!("{}×{}", s.size.cols, s.size.rows), UI_SMALL, TEXT_FAINT()));

    let lines = {
        let term = t.session.term.lock().unwrap();
        grid::lines(&term, s.focused)
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
            text(line.text).key(format!("l{i}")).font(TERM).color(TERM_FG()).spans(line.spans).h(LINE_H),
        );
    }

    col().key("terminal-card").w_full().h_full().radius(10.0).fill(TERM_BG()).border(1.0, LINE_STRONG()).clip_rounded(10.0)
        .child(header)
        .child(el().w_full().h(1.0).fill(LINE()))
        .child(col().grow(1.0).min_h(0.0).w_full().px(10.0).py(8.0).child(grid_el))
}

fn main() {
    rune::run(AppSpec::<App> {
        min_size: Some((720.0, 420.0)),
        families: ("Inter", "JetBrainsMono Nerd Font, JetBrains Mono, ui-monospace"),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: &[("three-tabs", |s| {
            s.open_tab(0);
            s.open_tab(0);
            s.open_tab(0);
        })],
        ..AppSpec::new("contr", (1280.0, 800.0), init, view)
    });
}
