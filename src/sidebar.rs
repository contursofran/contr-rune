//! The sidebar (K4): rune-kit's `Sidebar` and `SidebarRow`. A "Projects" head with `+` (open a
//! folder), each project with its terminals under it, then "Servers" and the settings gear.
//!
//! Under a project come its agents (H5): each orchestrator, then its workers one level deeper (the
//! order and nesting come from `hc::model::lines`), then the project's terminal tabs. H6 makes
//! their state live: the glyph per state, and the message of a needs-you row as its help.
//!
//! Element keys: `add-project`, `project-<i>`, `add-<i>` (the project's hover `+`),
//! `agent/<address>` (with `glyph` = the state mark, its label the state, `subtitle` = the meta line
//! and `detail` = the age inside),
//! `term-<id>`, `server-this-mac`, `settings`, and in the foot the plan meters `meter/claude` and `meter/codex`
//! (H11; each has the parts `label`, `bar` and `value`, the value is the percent used or "–").
//!
//! A secondary press on an agent row opens its menu (A1): Interrupt, Stop, Open PR. The kit names
//! the parts `agent/<address>/menu-menu` and `agent/<address>/menu-item0..2` (Interrupt, Stop, Open
//! PR; the items carry their label). The actions run off the UI thread; a failure shows in the
//! error line.
//!
//! The Projects `+` opens a menu (A3): `add-project/menu-menu` with `add-project/menu-item0` (New
//! orchestrator…, the folder picker, then `orchestrator::start`) and `add-project/menu-item1` (Open
//! folder…). A secondary press on an empty project row opens `project-<i>/menu-menu` with
//! `project-<i>/menu-item0` ("New orchestrator in <name>"). The spawn runs off the UI thread; the
//! row comes with the hc `created` event; a failure shows in the error line.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use rune::{Color, Cycle, El, FileDialogOptions, Icon, LoopSpec, PointerPress, el, icon, row, spacer, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarRow};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, Menu, MenuItem, Progress, context_menu};

use crate::github as gh;
use crate::hc::model::{self, Agent, AgentState, Line};
use crate::hc::{HcUpdate, watch};
use crate::orchestrator;
use crate::session::now_ms;
use crate::theme;
use crate::{App, PLUS, Project, Status, TERM, projects_modal, shell};

// Icons from main.html (16 point view box), as in `examples/kit_preview.rs`.
static GEAR: Icon = Icon {
    name: "contr-gear",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="2.2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/></g>"#,
};
static FOLDER: Icon = Icon {
    name: "contr-folder",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 4.5a1 1 0 0 1 1-1h3.6l1.5 1.6h5.9a1 1 0 0 1 1 1v6.4a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"/></g>"#,
};
// The Claude mark, as in `examples/kit_preview.rs`.
static CLAUDE: Icon = Icon {
    name: "contr-claude",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M9.19 8.17L14.93 8.97M8.95 8.74L12.10 11.20M8.45 9.11L10.40 13.93M7.83 9.19L7.33 12.75M7.26 8.95L3.81 13.36M6.89 8.45L2.99 10.02M6.81 7.83L1.86 7.14M7.05 7.26L4.06 4.92M7.55 6.89L5.38 1.51M8.17 6.81L8.72 2.85M8.74 7.05L11.69 3.27M9.11 7.55L12.54 6.16"/></g>"#,
};
static LAPTOP: Icon = Icon {
    name: "contr-laptop",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="10" height="7" rx="1.2"/><path d="M1.5 12.5h13"/></g>"#,
};

/// A terminal row is two lines (title, mono subtitle) under its project.
const TERMINAL_ROW_HEIGHT: f32 = 48.0;

/// An orchestrator row is two lines at 48; a worker row is 44 (as the kit preview).
const ORCHESTRATOR_ROW_HEIGHT: f32 = 48.0;
const WORKER_ROW_HEIGHT: f32 = 44.0;

/// The branch meta gets at most this width and this many characters; the project name keeps the rest.
const META_MAX_W: f32 = 96.0;
const META_MAX_CHARS: usize = 14;

/// `s` cut to `max` characters, ending in `…` when it was longer.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// A project's name: its folder's name, `/` for the root.
pub fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
}

/// The branch a `HEAD` file names: `ref: refs/heads/feat/x` gives `feat/x`; a detached `HEAD`
/// gives its first 7 characters.
pub fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(r) => {
            let r = r.trim();
            Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_owned()).filter(|b| !b.is_empty())
        }
        None if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) => Some(head[..7].to_owned()),
        None => None,
    }
}

/// The current branch of the repo at `dir`, read from `.git/HEAD` (no git binary). `.git` may
/// be a file (`gitdir: <path>`, as in worktrees). `None` when `dir` is not a repo.
pub fn branch_of(dir: &Path) -> Option<String> {
    let dot_git = dir.join(".git");
    let git_dir = if dot_git.is_file() {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = Path::new(text.trim().strip_prefix("gitdir:")?.trim()).to_path_buf();
        if target.is_absolute() { target } else { dir.join(target) }
    } else {
        dot_git
    };
    parse_head(&std::fs::read_to_string(git_dir.join("HEAD")).ok()?)
}

/// The age since a terminal's last output: `now` under a minute, then `14m`, `2h`, `3d`.
pub fn fmt_age(secs: u64) -> String {
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

fn icon_button(i: &'static Icon, label: &str) -> IconButton {
    IconButton::new(i).variant(IconButtonVariant::GhostMuted).control_size(ControlSize::Large).label(label.to_owned())
}

/// A 28 high section head: the label in 12/500 faint, then an optional action.
fn section_head(k: &Kit, title: &str, action: Option<El>) -> El {
    let r = row()
        .w_full()
        .h(28.0)
        .pl(10.0)
        .pr(2.0)
        .center_y()
        .child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_faint))
        .child(spacer());
    match action {
        Some(a) => r.child(a),
        None => r,
    }
}

fn project_row(k: &Kit, s: &App, i: usize, p: &Project) -> El {
    let empty = s.project_tabs(i).next().is_none();
    let meta = if empty { Some("nothing open".to_owned()) } else { branch_of(&p.path) };
    // Selected when it is the project with no terminal showing (a terminal row takes it otherwise).
    let selected = s.selected_project == i && s.active.is_none();
    // The meta and the hover `+` share one slot at the row's right edge: the meta shows, and on
    // hover it gives way to the `+` (no layout shift, no space kept for a hidden button).
    let add = icon_button(&PLUS, "New terminal here")
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.open_tab(i))
        .build(k)
        .key(format!("add-{i}"))
        .pin(None, Some(0.0), Some(0.0), None)
        .group_opacity(0.0, 1.0);
    let mut slot = row().key("meta-slot").h(20.0).min_w(20.0).max_w(META_MAX_W).shrink().center_y();
    if let Some(m) = meta {
        slot = slot.child(
            text(clip(&m, META_MAX_CHARS)).key("detail").font(k.ty.small).color(k.c.text_faint).truncate().group_opacity(1.0, 0.0),
        );
    }
    SidebarRow::new(p.name.clone())
        .icon(&FOLDER)
        .height(theme::ROW_HEIGHT)
        .title_style(if empty { theme::BODY } else { theme::BODY_STRONG })
        .selected(selected)
        .trailing(slot.child(add))
        .on_select(move |s: &mut App, _tx| s.select_project(i))
        .build(k)
        .key(format!("project-{i}"))
        .on_secondary_press(move |s: &mut App, at, _tx| {
            if s.project_tabs(i).next().is_none() && model::lines(&s.agents, i).is_empty() {
                s.agent_menu.project = Some((i, at));
            }
        })
}

/// The braille frames of a working glyph, as `main.html`'s `br` keyframes.
const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// One frame of the cycle: on for its tenth of the loop, off for the rest.
const FRAME_STOPS: &[(f32, f32)] = &[(0.0, 1.0), (0.099, 1.0), (0.1, 0.0), (1.0, 0.0)];
const BRAILLE_PERIOD: f32 = 0.9;
const GLYPH_W: f32 = 16.0;
const GLYPH_H: f32 = 18.0;

/// The kit's `Spinner` is a dot radar and `RowStatus` has no braille or pulse, so the design's
/// glyphs are drawn here with Rune's loop engine (renderer-owned: they run only on screen, share
/// one clock and hold still under reduced motion). The braille cycle is ten glyphs stacked, each
/// lit for a tenth of the 0.9 s loop; reduced motion holds the first.
fn braille(color: Color, label: &str) -> El {
    let mut b = el().key("glyph").role("img").label(label.to_owned()).size(GLYPH_W, GLYPH_H);
    for (i, g) in BRAILLE.iter().enumerate() {
        let cycle = Cycle { period: BRAILLE_PERIOD, phase: i as f32 / 10.0, stops: FRAME_STOPS, still: if i == 0 { 1.0 } else { 0.0 }, fps: 24.0 };
        b = b.child(row().key(format!("f{i}")).absolute(0.0, 0.0).size(GLYPH_W, GLYPH_H).center().child(text(*g).font(theme::GLYPH).color(color)).cycle(cycle));
    }
    b
}

/// The row's lead. A worker: braille cycling while working, red while it needs you, a pulsing
/// `⠿` while it waits, a faint `⠶` at rest. An orchestrator: the Claude mark, turning while it
/// works or waits, dim at rest. Red only for an agent that needs you. The element is keyed `glyph`
/// and says its state in its label (`needs you`, `working`, `waiting`, `idle`).
pub fn agent_glyph(k: &Kit, a: &Agent, orchestrates: bool) -> El {
    let word = match a.state {
        AgentState::NeedsYou { .. } => "needs you",
        AgentState::Working => "working",
        AgentState::Waiting => "waiting",
        AgentState::Idle | AgentState::Ended => "idle",
    };
    if a.is_orchestrator() {
        // A session that does not orchestrate draws the mark grey (D191).
        let hue = if orchestrates { theme::claude_color() } else { k.c.text_faint };
        let mark = icon(&CLAUDE, 16.0).key("glyph").label(word.to_owned());
        return match a.state {
            AgentState::NeedsYou { .. } => mark.color(k.c.danger.icon),
            AgentState::Working | AgentState::Waiting => mark.color(hue).looping(LoopSpec::rotation(2.4)),
            AgentState::Idle | AgentState::Ended => mark.color(hue).opacity(0.45),
        };
    }
    match a.state {
        AgentState::NeedsYou { .. } => braille(k.c.danger.icon, word),
        AgentState::Working => braille(k.c.text_base, word),
        AgentState::Waiting => text("⠿").key("glyph").label(word.to_owned()).font(theme::GLYPH).color(k.c.text_base).looping(LoopSpec::opacity(1.6, 0.0, 0.25, 1.0)),
        AgentState::Idle | AgentState::Ended => text("⠶").key("glyph").label(word.to_owned()).font(theme::GLYPH).color(k.c.text_faint),
    }
}

/// The mark of a plain terminal that runs Claude (D190): always grey, turning while it works.
pub fn plain_claude_glyph(k: &Kit, working: bool) -> El {
    let mark = icon(&CLAUDE, 16.0).key("glyph").label(if working { "working" } else { "idle" }.to_owned()).color(k.c.text_faint);
    if working { mark.looping(LoopSpec::rotation(2.4)) } else { mark.opacity(0.45) }
}

/// The open sidebar menus. The agent menu (A1): the row it belongs to, where it opened, and
/// whether that agent's branch has a pull request (`None` until `gh` answered). The Projects `+`
/// menu is open (A3). The menu of an empty project (A3): its index and where it opened.
#[derive(Default)]
pub struct AgentMenu {
    open: Option<(String, PointerPress)>,
    has_pr: Option<(String, bool)>,
    add_open: bool,
    pub project: Option<(usize, PointerPress)>,
}

/// What an agent action reports back to the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentDone {
    /// Whether the agent's branch has a pull request.
    Pr { address: String, available: bool },
    /// An action ended: Ok, or the text for the error line.
    Action(Result<(), String>),
}

#[derive(Clone, Copy)]
enum Action {
    Interrupt,
    Stop,
    OpenPr,
}

impl App {
    fn open_agent_menu(&mut self, address: String, at: PointerPress) {
        let folder = self.agents.iter().find(|a| a.address == address).and_then(|a| a.folder.clone());
        self.agent_menu = AgentMenu { open: Some((address.clone(), at)), has_pr: None, ..Default::default() };
        let Some(folder) = folder else { return };
        self.off_ui(move || AgentDone::Pr { address, available: gh::has_pr(&folder) });
    }

    fn agent_action(&mut self, address: String, action: Action) {
        self.agent_menu.open = None;
        let dialer = self.hc_dialer.clone();
        let folder = self.agents.iter().find(|a| a.address == address).and_then(|a| a.folder.clone());
        self.off_ui(move || {
            let hc = |f: fn(&crate::hc::Dialer, &str) -> Result<(), String>| match &dialer {
                Some(d) => f(d, &address),
                None => Err("hc is off".to_owned()),
            };
            AgentDone::Action(match action {
                Action::Interrupt => hc(watch::interrupt),
                Action::Stop => hc(watch::stop),
                Action::OpenPr => folder.ok_or_else(|| "the agent has no folder".to_owned()).and_then(|f| gh::open_pr(&f)),
            })
        });
    }

    /// Start an orchestrator in `dir` (A3), off the UI thread. A failure shows in the error line.
    fn start_orchestrator(&mut self, dir: PathBuf) {
        let (dialer, config) = (self.hc_dialer.clone(), crate::paths::agents_config());
        self.off_ui(move || AgentDone::Action(orchestrator::start(dialer.as_ref(), config, &dir, now_ms() / 1000).map_err(|e| format!("New orchestrator: {e}"))));
    }

    /// Run `job` off the UI thread and deliver its result as an update. A headless run cannot wait
    /// for a thread, so it runs at once.
    fn off_ui(&mut self, job: impl FnOnce() -> AgentDone + Send + 'static) {
        if rune::is_headless() {
            self.agent_done(job());
            return;
        }
        let (tx, waker) = (self.hc_tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send(HcUpdate::Agent(job()));
            if let Some(w) = &waker {
                w();
            }
        });
    }

    pub fn agent_done(&mut self, done: AgentDone) {
        match done {
            AgentDone::Pr { address, available } => self.agent_menu.has_pr = Some((address, available)),
            AgentDone::Action(Err(e)) => self.error = Some(e),
            AgentDone::Action(Ok(())) => {}
        }
    }
}

/// The open agent menu, mounted in `layer`.
fn agent_menu(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let Some((address, at)) = s.agent_menu.open.clone() else { return };
    let pr = s.agent_menu.has_pr.as_ref().is_some_and(|(a, ok)| *ok && *a == address);
    let item = |label: &str, action: Action| {
        let address = address.clone();
        MenuItem::new(label).on_select(move |s: &mut App| s.agent_action(address.clone(), action))
    };
    context_menu(format!("agent/{address}/menu"), el(), Some(at))
        .item(item("Interrupt", Action::Interrupt))
        .item(item("Stop", Action::Stop).danger(true))
        .item(item("Open PR", Action::OpenPr).disabled(!pr))
        .on_close(|s: &mut App| s.agent_menu.open = None)
        .build(k, layer);
}

/// The folder picker of the Projects `+` menu; `then` gets the folder.
pub fn pick_folder(tx: &mut rune::Tx, title: &str, then: fn(&mut App, PathBuf)) {
    let options = FileDialogOptions { title: title.into(), ..Default::default() };
    tx.open_folder(options, move |s: &mut App, result, _| match result {
        Ok(Some(path)) => then(s, path),
        Ok(None) => {}
        Err(e) => s.error = Some(format!("couldn't open a folder: {e}")),
    });
}

/// The menu of the empty project `i`, mounted in `layer`.
fn project_menu(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let Some((i, at)) = s.agent_menu.project else { return };
    let Some(p) = s.projects.get(i) else { return };
    let dir = p.path.clone();
    context_menu(format!("project-{i}/menu"), el(), Some(at))
        .item(MenuItem::new(format!("New orchestrator in {}", p.name)).on_select(move |s: &mut App| {
            s.agent_menu.project = None;
            s.start_orchestrator(dir.clone());
        }))
        .on_close(|s: &mut App| s.agent_menu.project = None)
        .build(k, layer);
}

fn agent_row(k: &Kit, s: &App, line: &Line) -> El {
    let a = line.agent;
    let address = a.address.clone();
    let mut meta = model::meta(line);
    if s.gh.ci(a) {
        meta.push_str("  CI");
    }
    let row = SidebarRow::new(s.gh.title(&a.address).unwrap_or(a.display_title()).to_owned())
        .avatar(agent_glyph(k, a, a.orchestrates(&s.agents)))
        .subtitle(meta)
        .subtitle_style(theme::META)
        .title_style(theme::BODY)
        .detail(a.age_ms.map(|ms| fmt_age(ms / 1000)).unwrap_or_default())
        .height(if line.depth <= 1 { ORCHESTRATOR_ROW_HEIGHT } else { WORKER_ROW_HEIGHT })
        .depth(line.depth)
        .selected(s.selected_agent.as_deref() == Some(a.address.as_str()))
        .on_select(move |s: &mut App, _tx| s.select_agent(address.clone()))
        .build(k)
        .key(format!("agent/{}", a.address))
        .on_secondary_press({
            let address = a.address.clone();
            move |s: &mut App, at, _tx| s.open_agent_menu(address.clone(), at)
        });
    // A row that needs you says why on hover.
    match &a.state {
        AgentState::NeedsYou { msg, .. } if !msg.trim().is_empty() => row.help(msg.clone()),
        _ => row,
    }
}

fn terminal_row(k: &Kit, s: &App, t: &crate::Tab) -> El {
    let id = t.session.id();
    let age = now_ms().saturating_sub(t.session.last_output().load(Ordering::Relaxed)) / 1000;
    let mut r = SidebarRow::new(t.session.tab_title())
        .avatar(icon(&TERM, 16.0).color(k.c.icon_muted))
        .subtitle_style(theme::META)
        .title_style(theme::BODY)
        .detail(fmt_age(age))
        .height(TERMINAL_ROW_HEIGHT)
        .depth(1)
        .selected(s.active == Some(id))
        .on_select(move |s: &mut App, _tx| s.select_tab(id));
    if let Some(sub) = t.session.subtitle() {
        r = r.subtitle(sub);
    }
    if s.tab_needs_you(id) {
        r = r.status(RowStatus::Attention);
    } else if s.status(id) == Status::Working {
        r = r.status(RowStatus::Busy);
    }
    r.build(k).key(format!("term-{id}"))
}

/// One plan meter of the foot: "Claude ▬ 38%". `used` is the percent used, none for no data ("–" and an empty bar).
fn plan_meter(k: &Kit, name: &str, key: &str, used: Option<u32>) -> El {
    let c = &k.c;
    let value = used.map_or("–".to_owned(), |p| format!("{p}%"));
    let bar = el().w(PLAN_BAR_WIDTH).child(Progress::new(Some(used.unwrap_or(0) as f32 / 100.0)).build(k)).key("bar");
    row()
        .key(key)
        .gap(6.0)
        .center_y()
        .child(text(name.to_owned()).key("label").font(k.ty.text_xsmall).color(c.text_faint))
        .child(bar)
        .child(text(value).key("value").font(k.ty.mono).color(c.text_muted))
}

/// Width of a plan meter's bar (Go: 44 px).
const PLAN_BAR_WIDTH: f32 = 44.0;

pub fn view(k: &Kit, s: &App, layer: &mut Vec<El>) -> El {
    agent_menu(k, s, layer);
    project_menu(k, s, layer);
    let plus = icon_button(&PLUS, "Open a folder").on_press(|s: &mut App| s.agent_menu.add_open = true).build(k).key("add-project");
    let add_project = Menu::new("add-project/menu")
        .trigger(plus)
        .open(s.agent_menu.add_open)
        .item(MenuItem::new("New orchestrator…").on_select_tx(|s: &mut App, tx| {
            s.agent_menu.add_open = false;
            pick_folder(tx, "Start an orchestrator in…", |s, path| {
                s.add_project(path.clone());
                s.start_orchestrator(path);
            });
        }))
        .item(MenuItem::new("Open folder…").on_select_tx(|s: &mut App, tx| {
            s.agent_menu.add_open = false;
            pick_folder(tx, "Open a folder", App::add_project);
        }))
        .item(MenuItem::new("Open projects…").on_select(|s: &mut App| {
            s.agent_menu.add_open = false;
            s.open_projects_modal();
        }))
        .on_close(|s: &mut App| s.agent_menu.add_open = false)
        .build(k, layer);
    let mut list = Sidebar::new().label("Projects").inset(0.0).row_gap(2.0).header(section_head(k, "Projects", Some(add_project)));
    for (n, i) in projects_modal::sidebar_order(s).into_iter().enumerate() {
        let p = &s.projects[i];
        if n > 0 {
            list = list.child(el().h(10.0));
        }
        list = list.child(project_row(k, s, i, p));
        for line in model::lines(&s.agents, i) {
            list = list.child(agent_row(k, s, &line));
        }
        // An agent's tab is shown by its agent row only.
        for t in s.project_tabs(i).filter(|t| t.agent.is_none()) {
            list = list.child(terminal_row(k, s, t));
        }
    }
    let this_mac = SidebarRow::new("this Mac").icon(&LAPTOP).detail(crate::host::meta(s.host.reading)).height(theme::ROW_HEIGHT).title_style(theme::BODY).selected(true).build(k).key("server-this-mac");
    let gear = icon_button(&GEAR, "Settings").on_press(|s: &mut App| s.settings_open = true).build(k).key("settings");
    if let Some(e) = &s.error {
        list = list.footer(text(e.clone()).font(k.ty.text_xsmall).color(k.c.danger.icon).key("error").px(10.0).pb(6.0));
    }
    let list = list
        .footer(section_head(k, "Servers", None))
        .footer(this_mac)
        .footer(row().w_full().h(1.0).fill(k.c.border_muted).mt(6.0))
        .footer(
            row()
                .w_full()
                .pt(8.0)
                .pr(2.0)
                .gap(12.0)
                .center_y()
                .child(plan_meter(k, "Claude", "meter/claude", s.plans.reading.claude))
                .child(plan_meter(k, "Codex", "meter/codex", s.plans.reading.codex))
                .child(spacer())
                .child(gear),
        )
        .build(k);
    shell::sidebar_panel(k).padding(10.0).child(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_format() {
        assert_eq!(fmt_age(0), "now");
        assert_eq!(fmt_age(59), "now");
        assert_eq!(fmt_age(60), "1m");
        assert_eq!(fmt_age(14 * 60 + 5), "14m");
        assert_eq!(fmt_age(3599), "59m");
        assert_eq!(fmt_age(3600), "1h");
        assert_eq!(fmt_age(2 * 3600 + 900), "2h");
        assert_eq!(fmt_age(86_400 * 3), "3d");
    }

    #[test]
    fn head_parsing() {
        assert_eq!(parse_head("ref: refs/heads/main\n"), Some("main".into()));
        assert_eq!(parse_head("ref: refs/heads/feat/x"), Some("feat/x".into()));
        assert_eq!(parse_head("ref: refs/remotes/o/m"), Some("refs/remotes/o/m".into()));
        assert_eq!(parse_head("0123456789abcdef0123456789abcdef01234567\n"), Some("0123456".into()));
        assert_eq!(parse_head("ref: \n"), None);
        assert_eq!(parse_head("garbage"), None);
        assert_eq!(parse_head(""), None);
    }

    #[test]
    fn branch_reads_head_from_the_repo() {
        let dir = std::env::temp_dir().join(format!("contr-rune-branch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        assert_eq!(branch_of(&dir), None, "no HEAD yet");
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        assert_eq!(branch_of(&dir), Some("dev".into()));
        // A worktree: `.git` is a file that points at the repo's dir.
        let wt = dir.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../.git\n").unwrap();
        assert_eq!(branch_of(&wt), Some("dev".into()));
        // Not a repo.
        assert_eq!(branch_of(&dir.join("nope")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_text_is_clipped_with_an_ellipsis() {
        assert_eq!(clip("main", 14), "main");
        assert_eq!(clip("worktree-agent-a562bb76be1982d77", 14), "worktree-agen…");
        assert_eq!(clip("worktree-agen", 13), "worktree-agen");
        assert_eq!(clip("ééé", 2), "é…");
    }

    #[test]
    fn folder_names() {
        assert_eq!(folder_name(Path::new("/a/b")), "b");
        assert_eq!(folder_name(Path::new("/")), "/");
    }
}
