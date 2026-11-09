//! contr-rune: contr's sidebar, terminal tabs and terminal, rebuilt on Rune.
//!
//! Left: projects, each with its terminals, then the servers. Top: the tabs of the selected project and `+`. Middle: the selected
//! terminal, a real shell in a PTY.
//!
//! `CONTR_PROJECTS=~/a:~/b cargo run` opens those folders as projects (default: the current
//! folder and home). Shortcuts: Ctrl+Shift+T new tab, Ctrl+Shift+W close, Ctrl+Tab /
//! Ctrl+Shift+Tab or Ctrl+PageDown / PageUp switch, Ctrl+Shift+V paste (⌘ on macOS), Ctrl+Shift+F find
//! (⌘F; ⌘G / ⌘⇧G or Return / Shift+Return step, esc closes), Ctrl+Shift+D split (⌘D), Ctrl+Shift+[ / ]
//! focus the left / right pane (⌘[ / ⌘]), Ctrl+Shift+J the questions card (⌘J).

mod agents;
mod agents_config;
mod cli;
mod dialog;
mod diff;
mod files;
mod find;
mod flow_section;
mod github;
mod grid;
mod hc;
mod host;
mod keys;
mod notch;
mod orchestrator;
mod outline;
mod paths;
mod projects;
mod projects_modal;
mod prompt;
mod pty;
mod select;
mod session;
mod session_file;
mod session_panel;
mod settings;
mod settings_agents;
mod shell;
mod sidebar;
mod split;
mod tabs;
mod terminal_view;
mod theme;
mod titlebar;
mod update;
mod usage;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use rune_kit::shell::TabDragState;
use rune_kit::Kit;
use rune::{AppSpec, Color, Cx, El, Icon, Key, KeyEvent, Tx, Waker, text};

use diff::DiffSummary;
use files::{Scan, files_panel};
use find::Find;
use hc::term::{HcSession, Ownership, Spec as HcSpec};
use hc::model::{self, Agent};
use hc::owner::{OwnerFile, OwnerSource, OwnerState};
use hc::restore;
use hc::{Dialer, HcStatus, HcUpdate};
use pty::Pty;
use session::{Session, Size, now_ms};
use settings::Appearance;
use split::Split;
use terminal_view::terminal_panel;
use theme::*;
use titlebar::title_bar;

/// The diff summary is refreshed at most this often (a new project selection refreshes at once).
const DIFF_EVERY: Duration = Duration::from_secs(3);

/// How often `owner.json` is checked for changes while the window has the focus.
const OWNER_EVERY: Duration = Duration::from_secs(1);

/// Output within this long ago makes a terminal "working".
const WORKING_MS: u64 = 1_500;

struct Project {
    name: String,
    path: PathBuf,
    /// The tab to show when the project is selected again.
    last_tab: Option<u64>,
    /// Taken off the sidebar in the projects modal (C6); its terminals keep running.
    hidden: bool,
    /// The worker whose tab shows in the project's set (T4): the last one selected. A plain tab keeps
    /// it; a Claude session's tab drops it.
    last_worker: Option<String>,
}

struct Tab {
    project: usize,
    session: Pty,
    /// The address of the agent whose terminal this tab shows (H8). Such a tab is not owned by the
    /// app: closing it only detaches. Plain terminal tabs have `None` and are killed on close.
    agent: Option<String>,
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
    /// Terminal size of each pane's laid-out grid area (the second is used while split).
    pane_size: [Size; 2],
    /// Two panes side by side (⌘D); `None` shows `active` alone.
    split: Option<Split>,
    /// The split row's laid-out width, for the divider drag.
    split_total: f32,
    wheel: f32,
    statuses: Vec<(u64, Status)>,
    focused: bool,
    error: Option<String>,
    /// Drag state of the title bar's tab strip.
    tab_drag: TabDragState,
    /// Dark, light or the system's; the view builds its one kit from it.
    appearance: Appearance,
    /// The settings dialog is open.
    settings_open: bool,
    /// The tab of the settings dialog.
    settings_tab: dialog::Tab,
    /// The Roles of the Agents tab.
    roles: settings_agents::Roles,
    /// Whether the files panel is shown (⌘B).
    show_files: bool,
    /// The selected project's working-tree summary; `None` when it is not a repo or has no changes.
    diff: Option<DiffSummary>,
    /// The project path `diff` belongs to.
    diff_for: Option<PathBuf>,
    /// A refresh is wanted (a terminal went idle, or the project changed).
    diff_stale: bool,
    /// A git run is in flight.
    diff_inflight: bool,
    diff_last: Option<Instant>,
    diff_tx: Sender<(PathBuf, Scan)>,
    diff_rx: Receiver<(PathBuf, Scan)>,
    /// The selected project's file tree (from the same scan as `diff`).
    tree: files::Model,
    /// The files panel shows only changed files and their folders.
    changed_only: bool,
    /// The find bar over the terminal (⌘F).
    find: Find,
    /// The connection to hc (herdr-core), shown in Settings.
    hc: HcStatus,
    hc_tx: Sender<HcUpdate>,
    hc_rx: Receiver<HcUpdate>,
    /// How to reach hc for tabs: set when hc is on and installed (or the fake core is chosen);
    /// `None` means local PTY tabs.
    hc_dialer: Option<Dialer>,
    /// The tab whose terminal was last told it has the keyboard focus (FOCUS frames).
    focus_sent: Option<u64>,
    /// Every terminal hc has and when each last had an event (from the watcher), the agents derived
    /// from them (`refresh_agents`) and the minute those ages were computed in.
    hc_terminals: Vec<hc::wire::Terminal>,
    hc_stamps: HashMap<String, u64>,
    agents: Vec<Agent>,
    agents_minute: u64,
    /// Issue titles and CI state of the agents, read by `gh` (A4).
    gh: github::Info,
    /// The agent row that is selected (its address). Showing its terminal comes in H8.
    selected_agent: Option<String>,
    /// Claude sessions whose tab was closed: they stay out of the set until selected in the sidebar.
    hidden: HashSet<String>,
    agent_menu: sidebar::AgentMenu,
    /// The open tab menu (T3): the tab and where it opened.
    tab_menu: Option<(u64, rune::PointerPress)>,
    /// The owner's questions and to-dos (`owner.json`, shared with Go contr), what was last read from
    /// it, when that file was last changed, and the card that shows them (H9).
    owner: Option<Arc<dyn OwnerSource>>,
    owner_state: OwnerState,
    owner_stamp: Option<std::time::SystemTime>,
    card: notch::Card,
    /// Claude and Codex plan meters in the sidebar foot (H11).
    plans: usage::Plans,
    host: host::Host,
    context: usage::ContextCache,
    session: session_panel::SessionCache,
    /// The terminal text selection (T1).
    select: select::Select,
    /// The update check, pill and toast (R3a).
    update: update::Updater,
    /// The orchestrator outline (H10).
    outline: outline::State,
    /// The projects modal (C6) and the recent list it shows.
    pm: projects_modal::State,
    recent: Vec<projects::Recent>,
}

fn init() -> App {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    // `CONTR_PROJECTS` wins; else the saved projects (C5); else the folder we started in and home.
    let mut saved_selected = None;
    let mut hidden_paths = Vec::new();
    let mut recent = Vec::new();
    let paths: Vec<PathBuf> = match std::env::var("CONTR_PROJECTS").ok().filter(|l| !l.is_empty()) {
        Some(list) => list
            .split(':')
            .filter(|p| !p.is_empty())
            .map(|p| match p.strip_prefix("~/") { Some(rest) => home.join(rest), None => PathBuf::from(p) })
            .filter(|p| p.is_dir())
            .collect(),
        None => match projects::load() {
            Some(saved) => {
                saved_selected = saved.selected;
                hidden_paths = saved.hidden;
                recent = saved.recent;
                saved.open
            }
            None => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
                if cwd == home { vec![home] } else { vec![cwd, home] }
            }
        },
    };
    let selected_project = saved_selected.and_then(|s| paths.iter().position(|p| *p == s)).unwrap_or(0);
    let hidden_paths = hidden_paths.into_iter().filter(|h| !paths.contains(h));
    let projects = paths
        .iter()
        .cloned()
        .map(|path| (path, false))
        .chain(hidden_paths.map(|path| (path, true)))
        .map(|(path, hidden)| Project {
            name: sidebar::folder_name(&path),
            path,
            last_tab: None, last_worker: None,
            hidden,
        })
        .collect();
    let (diff_tx, diff_rx) = channel();
    let (hc_tx, hc_rx) = channel();
    // Chosen here, not in `setup`: a scene opens its tabs before `setup` runs.
    let (hc_dialer, hc) = match Dialer::from_env(rune::is_headless()) {
        Ok(d) => (Some(d), HcStatus::Connecting),
        Err(status) => (None, status),
    };
    // A headless run reads only the file it is given: never the owner's real one.
    let owner: Option<Arc<dyn OwnerSource>> = match OwnerFile::from_env() {
        Some(f) if !rune::is_headless() || std::env::var_os("CONTR_OWNER_FILE").is_some() => Some(Arc::new(f)),
        _ => None,
    };
    let mut app = App {
        projects,
        tabs: Vec::new(),
        selected_project,
        active: None,
        waker: None,
        next_id: 1,
        pane_size: [Size { cols: 100, rows: 30 }; 2],
        split: None,
        split_total: 0.0,
        wheel: 0.0,
        statuses: Vec::new(),
        focused: true,
        error: None,
        tab_drag: TabDragState::new(),
        appearance: settings::load(),
        settings_open: false,
        settings_tab: dialog::Tab::General,
        roles: settings_agents::Roles::default(),
        show_files: true,
        diff: None,
        diff_for: None,
        diff_stale: false,
        diff_inflight: false,
        diff_last: None,
        diff_tx,
        diff_rx,
        tree: files::empty(),
        changed_only: false,
        hc,
        hc_tx,
        hc_rx,
        hc_dialer,
        focus_sent: None,
        hc_terminals: Vec::new(),
        hc_stamps: HashMap::new(),
        agents: Vec::new(),
        agents_minute: 0,
        gh: github::Info::new(),
        selected_agent: None,
        hidden: HashSet::new(),
        agent_menu: Default::default(),
        tab_menu: None,
        find: Find::new(),
        owner,
        owner_state: OwnerState::default(),
        owner_stamp: None,
        card: notch::Card::default(),
        plans: usage::Plans::new(),
        host: host::Host::new(),
        context: usage::ContextCache::default(),
        session: session_panel::SessionCache::default(),
        select: select::Select::default(),
        update: update::Updater::new(),
        outline: outline::State::default(),
        pm: projects_modal::State::default(),
        recent,
    };
    app.reload_owner(true);
    app
}

impl App {
    fn open_tab(&mut self, project: usize) {
        let Some(p) = self.projects.get(project) else { return };
        let id = self.next_id;
        let size = self.pane_size[self.focus_idx()];
        // With hc the tab is a terminal in the core; a thread creates it and a failure falls back
        // to a local PTY (see `hc_fallbacks`).
        if let Some(dialer) = self.hc_dialer.clone() {
            let spec = HcSpec { dialer, cwd: p.path.clone(), project: p.name.clone(), size, waker: self.waker.clone(), existing: None, ownership: Ownership::Owned };
            self.next_id += 1;
            self.tabs.push(Tab { project, session: Pty::Hc(HcSession::start(id, spec)), agent: None });
            self.select_tab(id);
            return;
        }
        match Session::spawn(id, &p.path, size, self.waker.clone()) {
            Ok(session) => {
                self.next_id += 1;
                self.tabs.push(Tab { project, session: Pty::Local(session), agent: None });
                self.select_tab(id);
            }
            Err(e) => self.error = Some(format!("couldn't start a shell: {e}")),
        }
    }

    /// Reopen the app's live hc terminals as tabs (launch): each in its project, in creation order,
    /// attached with history. A folder that is no project yet is added when it exists. With nothing
    /// to reopen (or no answer from hc) one new tab opens, as without hc.
    fn restore_tabs(&mut self, listed: Result<Vec<hc::wire::Terminal>, String>) {
        let Some(dialer) = self.hc_dialer.clone() else { return };
        let terminals = listed.unwrap_or_default();
        let same = same_path;
        let items = {
            let known: Vec<restore::Known> = self.projects.iter().map(|p| restore::Known { name: &p.name, path: &p.path }).collect();
            restore::plan(&terminals, &known, &same, &|p| p.is_dir())
        };
        let size = self.pane_size[0];
        let mut opened: Vec<(u64, usize)> = Vec::new();
        for item in items {
            let project = match item.target {
                restore::Target::Project(i) => i,
                restore::Target::Folder(path) => match self.projects.iter().position(|p| same(&p.path, &path)) {
                    Some(i) => i,
                    None => {
                        self.projects.push(Project { name: sidebar::folder_name(&path), path, last_tab: None, hidden: false, last_worker: None });
                        self.projects.len() - 1
                    }
                },
            };
            let p = &self.projects[project];
            let spec = HcSpec { dialer: dialer.clone(), cwd: p.path.clone(), project: p.name.clone(), size, waker: self.waker.clone(), existing: Some(item.address), ownership: Ownership::Owned };
            let id = self.next_id;
            self.next_id += 1;
            self.tabs.push(Tab { project, session: Pty::Hc(HcSession::start(id, spec)), agent: None });
            opened.push((id, project));
        }
        self.refresh_agents();
        // The first tab of the selected project shows; else the first one.
        match opened.iter().find(|(_, p)| *p == self.selected_project).or(opened.first()) {
            Some((id, _)) => self.select_tab(*id),
            None if !self.tabs.iter().any(|t| t.agent.is_none()) && !self.projects.is_empty() => self.open_tab(self.selected_project),
            None => {}
        }
    }

    /// Tell the terminals which pane has the keyboard focus: a FOCUS frame when it changes.
    fn sync_focus(&mut self) {
        let want = if self.focused { self.pane_tab(self.focus_idx()) } else { None };
        if want == self.focus_sent {
            return;
        }
        for (id, gained) in [(self.focus_sent, false), (want, true)] {
            if let Some(id) = id
                && let Some(t) = self.tabs.iter().find(|t| t.session.id() == id)
            {
                t.session.set_focus(gained);
            }
        }
        self.focus_sent = want;
    }

    /// Add a folder as a project and select it; a folder that is already a project is just selected.
    fn add_project(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        let i = match self.projects.iter().position(|p| p.path == path) {
            Some(i) => {
                self.projects[i].hidden = false;
                i
            }
            None => {
                self.projects.push(Project { name: sidebar::folder_name(&path), path, last_tab: None, hidden: false, last_worker: None });
                self.projects.len() - 1
            }
        };
        self.refresh_agents();
        self.select_project(i);
    }

    /// Take the terminal list from the watcher and derive the agents again.
    fn set_terminals(&mut self, terminals: Vec<hc::wire::Terminal>, stamps: HashMap<String, u64>) {
        self.hc_terminals = terminals;
        self.hc_stamps = stamps;
        self.refresh_agents();
        self.sync_menu();
    }

    /// Derive the agents from the terminal list. An agent whose project is not open opens it when
    /// its folder exists (as H4 does for tabs); one whose folder is gone is not shown.
    fn refresh_agents(&mut self) {
        let now = now_ms();
        for _ in 0..2 {
            let agents = {
                let known: Vec<restore::Known> = self.projects.iter().map(|p| restore::Known { name: &p.name, path: &p.path }).collect();
                model::derive(&self.hc_terminals, &self.hc_stamps, &known, &within_path, now)
            };
            let mut added = false;
            for a in agents.iter().filter(|a| a.project.is_none() && a.state != model::AgentState::Ended) {
                let Some(path) = a.folder.clone().filter(|f| f.is_dir()) else { continue };
                if self.projects.iter().any(|p| same_path(&p.path, &path)) { continue; }
                self.projects.push(Project { name: sidebar::folder_name(&path), path, last_tab: None, hidden: false, last_worker: None });
                added = true;
            }
            self.agents = agents;
            if !added { break; }
        }
        self.agents_minute = now / 60_000;
        // A worker that is gone cannot be kept in the set.
        let known: Vec<&str> = self.agents.iter().filter(|a| a.state != model::AgentState::Ended).map(|a| a.address.as_str()).collect();
        for p in &mut self.projects {
            if p.last_worker.as_deref().is_some_and(|w| !known.contains(&w)) { p.last_worker = None; }
        }
        self.sync_tabs();
        if self.selected_agent.as_ref().is_some_and(|a| !self.agents.iter().any(|x| &x.address == a)) {
            self.selected_agent = None;
        }
    }

    /// Show an agent's terminal: select the tab that already shows it, else attach it as a tab of the
    /// agent's project. A Claude session the owner closed comes back.
    fn select_agent(&mut self, address: String) {
        self.hidden.remove(&address);
        let id = match self.tabs.iter().find(|t| t.agent.as_deref() == Some(address.as_str())).map(|t| t.session.id()) {
            Some(id) => id,
            None => match self.attach_agent(address) {
                Some(id) => id,
                None => return,
            },
        };
        self.select_tab(id);
    }

    /// Attach an agent's terminal (framed, with history, read-write) as a tab of its project, in the
    /// focused pane's size. The tab is not owned by the app (`Ownership::Borrowed`): closing it only
    /// detaches. It is not selected.
    fn attach_agent(&mut self, address: String) -> Option<u64> {
        let project = self.agents.iter().find(|a| a.address == address).and_then(|a| a.project)?;
        let dialer = self.hc_dialer.clone()?;
        let p = self.projects.get(project)?;
        let id = self.next_id;
        self.next_id += 1;
        let size = self.pane_size[self.focus_idx()];
        let spec = HcSpec { dialer, cwd: p.path.clone(), project: p.name.clone(), size, waker: self.waker.clone(), existing: Some(address.clone()), ownership: Ownership::Borrowed };
        self.tabs.push(Tab { project, session: Pty::Hc(HcSession::start(id, spec)), agent: Some(address) });
        Some(id)
    }

    /// What a tab shows in the set (T4).
    fn tab_kind(&self, t: &Tab) -> tabs::Kind<'_> {
        match t.agent.as_deref().and_then(|addr| self.agents.iter().find(|a| a.address == addr)) {
            Some(a) if a.is_orchestrator() => tabs::Kind::Claude(self.sidebar_place(a)),
            Some(a) => tabs::Kind::Worker(&a.address),
            None if self.tab_runs_claude(t.session.id()) => tabs::Kind::Claude(usize::MAX),
            None => tabs::Kind::Plain,
        }
    }

    /// The place of an agent among its project's sidebar rows.
    fn sidebar_place(&self, a: &hc::model::Agent) -> usize {
        a.project.and_then(|p| model::lines(&self.agents, p).iter().position(|l| l.agent.address == a.address)).unwrap_or(usize::MAX)
    }

    /// A plain tab whose terminal runs Claude Code (D190).
    fn tab_runs_claude(&self, id: u64) -> bool {
        let Some(address) = self.tabs.iter().find(|t| t.session.id() == id).filter(|t| t.agent.is_none()).and_then(|t| t.session.address()) else { return false };
        self.hc_terminals.iter().any(|t| t.address == address && t.state != "finished" && model::runs_claude(t))
    }

    /// Bring the selected project's tabs to the shared set (T4): attach every Claude session that is
    /// not hidden, detach the workers other than the kept one, and put the tabs in order.
    fn sync_tabs(&mut self) {
        let project = self.selected_project;
        if project >= self.projects.len() { return; }
        let wanted: Vec<String> = model::lines(&self.agents, project)
            .iter()
            .filter(|l| l.agent.is_orchestrator() && !self.hidden.contains(&l.agent.address))
            .map(|l| l.agent.address.clone())
            .filter(|addr| !self.tabs.iter().any(|t| t.agent.as_deref() == Some(addr.as_str())))
            .collect();
        for address in wanted {
            self.attach_agent(address);
        }
        let slots: Vec<usize> = self.tabs.iter().enumerate().filter(|(_, t)| t.project == project).map(|(i, _)| i).collect();
        let kinds: Vec<tabs::Kind> = slots.iter().map(|i| self.tab_kind(&self.tabs[*i])).collect();
        let order = tabs::order(&kinds, self.projects[project].last_worker.as_deref());
        // Workers that are not kept detach (the tab is not owned: its terminal lives on).
        let mut taken: Vec<Option<Tab>> = slots.iter().rev().map(|i| Some(self.tabs.remove(*i))).collect();
        taken.reverse();
        let mut ordered: Vec<Tab> = order.iter().filter_map(|i| taken[*i].take()).collect();
        for t in taken.into_iter().flatten() {
            t.session.close();
        }
        for (slot, t) in slots.iter().zip(ordered.drain(..)) {
            self.tabs.insert(*slot, t);
        }
    }

    fn select_tab(&mut self, id: u64) {
        let Some(project) = self.tabs.iter().find(|t| t.session.id() == id).map(|t| t.project) else { return };
        // A worker's tab becomes the project's kept worker, a Claude session's drops it, a plain tab
        // keeps it (T4).
        let kept = match self.tabs.iter().find(|t| t.session.id() == id).map(|t| self.tab_kind(t)) {
            Some(tabs::Kind::Worker(a)) => Some(Some(a.to_owned())),
            Some(tabs::Kind::Claude(_)) => Some(None),
            _ => None,
        };
        if let Some(kept) = kept { self.projects[project].last_worker = kept; }
        // Selecting a plain tab clears the agent selection; an agent's tab sets it.
        self.selected_agent = self.tabs.iter().find(|t| t.session.id() == id).and_then(|t| t.agent.clone());
        // A split belongs to one project: a tab of another project takes the whole panel.
        if self.split.as_ref().is_some_and(|sp| sp.project != project) { self.split = None; }
        // In a split the tab goes to the focused pane, or, when the other pane shows it, focus moves there.
        if let Some(sp) = &mut self.split {
            match sp.panes.iter().position(|p| *p == id) {
                Some(i) => sp.focus = i,
                None => sp.panes[sp.focus] = id,
            }
        }
        let size = self.pane_size[self.focus_idx()];
        if let Some(t) = self.tabs.iter_mut().find(|t| t.session.id() == id) { t.session.resize(size); }
        self.selected_project = project;
        self.projects[project].last_tab = Some(id);
        // The find bar belongs to one terminal: another tab starts clean.
        if self.active != Some(id) { self.find.close(); }
        self.active = Some(id);
        self.sync_tabs();
    }

    fn select_project(&mut self, project: usize) {
        self.selected_agent = None;
        self.selected_project = project;
        let last = self.projects[project].last_tab.filter(|id| self.tabs.iter().any(|t| t.session.id() == *id));
        match last.or_else(|| self.project_tabs(project).next().map(|t| t.session.id())) {
            Some(id) => self.select_tab(id),
            None => { self.split = None; self.active = None; self.find.close(); }
        }
        self.sync_tabs();
        self.save_projects();
    }

    /// Keep the open projects and the selected one in the projects file (C5).
    fn save_projects(&mut self) {
        let paths = |hidden| self.projects.iter().filter(move |p| p.hidden == hidden).map(|p| p.path.clone()).collect();
        let saved = projects::Saved {
            open: paths(false),
            selected: self.projects.get(self.selected_project).map(|p| p.path.clone()),
            hidden: paths(true),
            recent: self.recent.clone(),
        };
        if let Err(e) = projects::save(&saved) {
            self.error = Some(format!("couldn't save the projects: {e}"));
        }
    }

    fn close_tab(&mut self, id: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.session.id() == id) else { return };
        let project = self.tabs[i].project;
        // Dropping a local session closes the PTY (the shell gets SIGHUP). An hc terminal we made is
        // killed and removed; an agent's tab (`agent` is set) only detaches.
        self.tabs[i].session.close();
        // A closed Claude session stays out of the set until it is selected in the sidebar.
        if let Some(address) = self.tabs[i].agent.clone() { self.hidden.insert(address); }
        let siblings: Vec<u64> = self.project_tabs(project).map(|t| t.session.id()).collect();
        let pos = siblings.iter().position(|s| *s == id).unwrap_or(0);
        // Closing either pane's terminal un-splits; the other one stays.
        let split = self.split.take_if(|sp| sp.panes.contains(&id));
        self.tabs.remove(i);
        if self.active == Some(id) {
            let rest: Vec<u64> = self.project_tabs(project).map(|t| t.session.id()).collect();
            let other = split.map(|sp| if sp.panes[0] == id { sp.panes[1] } else { sp.panes[0] });
            match other.or_else(|| rest.get(pos.min(rest.len().saturating_sub(1))).copied()) {
                Some(next) => self.select_tab(next),
                None => { self.active = None; self.selected_agent = None; self.find.close(); }
            }
        }
    }

    /// Close every other tab of `id`'s project (each by the usual close rules: a plain tab is killed
    /// and removed, an agent's tab only detaches), then show `id`.
    fn close_other_tabs(&mut self, id: u64) {
        let Some(project) = self.tabs.iter().find(|t| t.session.id() == id).map(|t| t.project) else { return };
        let others: Vec<u64> = self.project_tabs(project).map(|t| t.session.id()).filter(|t| *t != id).collect();
        for other in others {
            self.close_tab(other);
        }
        self.select_tab(id);
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
        // In a split the other pane's tab is skipped: it is already on screen.
        let hidden = self.split.as_ref().map(|sp| sp.panes[1 - sp.focus]);
        let ids: Vec<u64> = self.project_tabs(self.selected_project).map(|t| t.session.id()).filter(|i| Some(*i) != hidden).collect();
        if ids.is_empty() { return; }
        let cur = self.active.and_then(|a| ids.iter().position(|i| *i == a)).unwrap_or(0) as i32;
        let next = (cur + step).rem_euclid(ids.len() as i32) as usize;
        self.select_tab(ids[next]);
    }

    fn project_tabs(&self, project: usize) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(move |t| t.project == project)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tabs.iter().find(|t| t.session.id() == id))
    }

    fn status(&self, id: u64) -> Status {
        self.statuses.iter().find(|(i, _)| *i == id).map(|(_, s)| *s).unwrap_or(Status::Idle)
    }

    /// The program in an hc tab reports `blocked`: it waits for the owner.
    /// The agent a tab shows, when it is an agent's tab and the agent is still known.
    fn tab_agent(&self, id: u64) -> Option<&hc::model::Agent> {
        let address = self.tabs.iter().find(|t| t.session.id() == id)?.agent.as_deref()?;
        self.agents.iter().find(|a| a.address == address)
    }

    fn tab_needs_you(&self, id: u64) -> bool {
        let Some(address) = self.tabs.iter().find(|t| t.session.id() == id).and_then(|t| t.session.address()) else { return false };
        self.hc_terminals.iter().any(|t| t.address == address && t.state != "finished" && t.status.as_ref().is_some_and(|s| s.state == "blocked"))
    }

    /// The focused pane (always 0 without a split).
    fn focus_idx(&self) -> usize {
        self.split.as_ref().map_or(0, |sp| sp.focus)
    }

    /// The session shown in pane `idx`.
    fn pane_tab(&self, idx: usize) -> Option<u64> {
        match &self.split {
            Some(sp) => sp.panes.get(idx).copied(),
            None => if idx == 0 { self.active } else { None },
        }
    }

    /// Pane `idx`'s grid was laid out: its terminal gets that many columns and rows.
    fn set_grid_area(&mut self, idx: usize, w: f32, h: f32) {
        let size = grid_size(w, h);
        self.pane_size[idx] = size;
        if let Some(id) = self.pane_tab(idx)
            && let Some(t) = self.tabs.iter_mut().find(|t| t.session.id() == id)
        {
            t.session.resize(size);
        }
    }

    /// Split the panel in two (the current terminal left, a new one in the same project right), or
    /// un-split: the focused pane takes the whole panel and the other terminal stays as a tab.
    fn toggle_split(&mut self) {
        if self.split.take().is_some() { return; }
        let Some(left) = self.active else { return };
        let project = self.selected_project;
        let before = self.tabs.len();
        self.open_tab(project);
        if self.tabs.len() > before
            && let Some(right) = self.active
        {
            self.split = Some(Split::new(left, right, project));
        }
    }

    /// Give pane `idx` the focus: its tab becomes the selected one.
    fn focus_pane(&mut self, idx: usize) {
        if let Some(id) = self.split.as_ref().and_then(|sp| sp.panes.get(idx).copied()) {
            self.select_tab(id);
        }
    }

    /// Move the divider; the left pane's share stays inside the limits.
    fn set_split_frac(&mut self, frac: f32) {
        if let Some(sp) = &mut self.split { sp.frac = split::clamp_frac(frac); }
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.projects.get(self.selected_project).map(|p| p.path.clone())
    }

    /// Compute the summary and tree now, on this thread (scenes only: a headless run cannot wait for the thread).
    fn refresh_diff_now(&mut self) {
        self.diff_for = self.selected_path();
        if let Some(path) = self.diff_for.clone() {
            self.apply_scan(files::scan(&path));
        }
        self.diff_stale = false;
    }

    /// Take a finished scan: the title bar's summary and the files tree, which keeps its open
    /// folders and selection.
    fn apply_scan(&mut self, scan: Scan) {
        self.diff = scan.summary.filter(|d| d.files > 0);
        let mut model = scan.model;
        let old = std::mem::replace(&mut self.tree, files::empty());
        files::carry_state(&mut model, old);
        files::set_changed_only(&mut model, self.changed_only);
        self.tree = model;
    }

    fn toggle_changed_only(&mut self) {
        self.changed_only = !self.changed_only;
        files::set_changed_only(&mut self.tree, self.changed_only);
    }

    /// Run git on a thread; `poll` picks the result up.
    fn spawn_diff(&mut self) {
        let Some(path) = self.selected_path() else { return };
        self.diff_inflight = true;
        self.diff_stale = false;
        self.diff_last = Some(Instant::now());
        let (tx, waker) = (self.diff_tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send((path.clone(), files::scan(&path)));
            if let Some(w) = &waker { w(); }
        });
    }

    /// Take a new appearance and keep it in the settings file.
    fn set_appearance(&mut self, a: Appearance) {
        self.appearance = a;
        if let Err(e) = settings::save(a) {
            self.error = Some(format!("couldn't save the settings: {e}"));
        }
    }

    /// Show or hide the files panel; the terminal card takes the freed width.
    fn toggle_files(&mut self) {
        self.show_files = !self.show_files;
    }

    /// Open the find bar with focus in the input and the old query selected.
    fn open_find(&mut self) {
        if self.active.is_none() { return; }
        self.find.open = true;
        self.find.edit.focused = true;
        self.find.edit.anchor = 0;
        self.find.edit.focus = self.find.edit.text.len();
        self.find_retarget();
    }

    /// The query changed: the current match becomes the first one in view, and the view follows.
    fn find_retarget(&mut self) {
        let Some(t) = self.active_tab() else { return };
        let (cur, delta) = {
            let term = t.session.term().lock().unwrap();
            let m = self.find.matches(t.session.id(), t.session.generation().load(Ordering::Acquire), &term);
            if m.is_empty() { (0, 0) } else {
                let cur = find::first_in_view(&m, term.grid().display_offset());
                (cur, find::scroll_delta(&term, &m[cur]))
            }
        };
        if delta != 0 { t.session.scroll(delta); }
        self.find.current = cur;
    }

    /// Move to the next or previous match, wrapping, and scroll it into view.
    fn find_step(&mut self, forward: bool) {
        let Some(t) = self.active_tab() else { return };
        let (cur, delta) = {
            let term = t.session.term().lock().unwrap();
            let m = self.find.matches(t.session.id(), t.session.generation().load(Ordering::Acquire), &term);
            let Some(cur) = find::step(self.find.current, m.len(), forward) else { return };
            (cur, find::scroll_delta(&term, &m[cur]))
        };
        if delta != 0 { t.session.scroll(delta); }
        self.find.current = cur;
    }

    /// Keys of the find bar, from the input and from the window. True when the key was used.
    fn find_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if is_shortcut(ev, 'f') {
            self.open_find();
            return true;
        }
        if !self.find.open { return false; }
        // Return, esc and F3 belong to the shell while the input has no focus (after a click on the grid).
        let focused = self.find.edit.focused;
        // Next is ⌘G or Return; previous adds Shift. Off macOS the chord already holds Shift, so Alt marks previous. F3 works everywhere.
        let back_chord = if cfg!(target_os = "macos") { ev.mods.shift } else { ev.mods.alt };
        match ev.key {
            Key::Escape if focused => {
                self.find.close();
                tx.blur();
            }
            Key::Enter if focused => self.find_step(!ev.mods.shift),
            Key::Function(3) if focused => self.find_step(!ev.mods.shift),
            _ if is_shortcut(ev, 'g') => self.find_step(!back_chord),
            _ => return false,
        }
        true
    }

    fn paste(&mut self, text: &str) {
        let Some(t) = self.active_tab() else { return };
        let bracketed = t.session.term().lock().unwrap().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            t.session.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            t.session.write(text.as_bytes());
        }
    }
}

/// Columns and rows that fit the laid-out grid area (already inside the panel padding).
/// Whether two folders are the same one (symlinks resolved).
fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Whether `folder` is `project` or inside it.
fn within_path(project: &Path, folder: &Path) -> bool {
    folder.starts_with(project) || matches!((project.canonicalize(), folder.canonicalize()), (Ok(p), Ok(f)) if f.starts_with(&p))
}

fn grid_size(w: f32, h: f32) -> Size {
    Size { cols: ((w / CELL_W).floor() as u16).max(2), rows: ((h / LINE_H).floor() as u16).max(1) }
}

// ───────────────────────────── hooks ─────────────────────────────

fn setup(s: &mut App, waker: Waker) {
    s.waker = Some(waker.clone());
    if rune::is_headless() {
        s.plans.read_now();
        s.host.fix_from_env();
    }
    start_hc(s, waker.clone());
    // A headless run cannot wait for the watcher: take the first listing now.
    if rune::is_headless() && let Some(d) = s.hc_dialer.clone() {
        if let Ok(list) = hc::watch::list_all(&d) {
            s.set_terminals(list, HashMap::new());
        }
    }
    // Claude sessions attached by the first listing do not count: the plain tabs of the last run still come back.
    if !s.tabs.iter().any(|t| t.agent.is_none()) && !s.projects.is_empty() {
        match s.hc_dialer.clone() {
            // With hc the tabs of the last run come back; the listing decides whether a new one opens.
            Some(d) if rune::is_headless() => s.restore_tabs(restore::list(&d)),
            Some(d) => restore::spawn(d, s.hc_tx.clone(), waker),
            None => s.open_tab(s.selected_project),
        }
    }
}

/// Start the hc watcher for the dialer `init` chose (none: hc is off or not installed). A headless
/// run never dials a real hc: with `CONTR_HC_FAKE` it checks the fake core at once (a scene cannot
/// wait for a thread) and only watches events, otherwise hc shows as not installed.
fn start_hc(s: &mut App, waker: Waker) {
    let Some(d) = s.hc_dialer.clone() else { return };
    if rune::is_headless() {
        s.hc = hc::watch::probe(&d);
        hc::watch::spawn_events(d, s.hc_tx.clone(), waker);
    } else {
        s.hc = HcStatus::Connecting;
        hc::watch::spawn(d, s.hc_tx.clone(), waker);
    }
}

/// An hc tab whose terminal could not be created becomes a local PTY (same id, same place).
fn hc_fallbacks(s: &mut App) -> bool {
    let size = s.pane_size[0];
    let mut changed = false;
    for t in &mut s.tabs {
        let Pty::Hc(h) = &t.session else { continue };
        if h.take_failure().is_none() { continue; }
        let id = h.id;
        match Session::spawn(id, &s.projects[t.project].path, size, s.waker.clone()) {
            Ok(local) => t.session = Pty::Local(local),
            Err(e) => {
                s.error = Some(format!("couldn't start a shell: {e}"));
                h.exited.store(true, Ordering::Release);
            }
        }
        changed = true;
    }
    changed
}

/// A headless run drives the fake hc core's clock with the script's and holds each frame until the
/// core is quiet, so a fixture event and what the app does about it happen in script order.
fn poll(s: &mut App, t: f64, tx: &mut Tx) -> bool {
    let fake = if rune::is_headless() { s.hc_dialer.clone() } else { None };
    if let Some(d) = &fake {
        d.step_fake(t);
    }
    let changed = poll_app(s, tx);
    if let Some(d) = &fake {
        d.settle_fake();
    }
    changed
}

fn poll_app(s: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    while let Ok(update) = s.hc_rx.try_recv() {
        if let Some(d) = &s.hc_dialer {
            d.note_fake_activity();
        }
        match update {
            HcUpdate::Status(status) => {
                if s.hc != status {
                    s.hc = status;
                }
            }
            HcUpdate::Event { terminal, kind } => {
                for t in &s.tabs {
                    t.session.hc_event(&terminal, kind);
                }
            }
            HcUpdate::Restore(listed) => s.restore_tabs(listed),
            HcUpdate::Terminals { terminals, stamps } => s.set_terminals(terminals, stamps),
            HcUpdate::Told(result) => s.told(result),
            update @ (HcUpdate::Menu { .. } | HcUpdate::Chose { .. }) => s.card_update(update),
            HcUpdate::Agent(done) => s.agent_done(done),
        }
        changed = true;
    }
    // Agent ages show minutes: derive again when the minute turns, and look again then.
    if !s.agents.is_empty() {
        if now_ms() / 60_000 != s.agents_minute {
            s.refresh_agents();
            changed = true;
        }
        tx.wake_after(Duration::from_secs(30));
    }
    changed |= hc_fallbacks(s);
    changed |= select::sweep(s);
    // The owner's file: look at it every second while the window has the focus, and not at all otherwise.
    if s.focused && s.owner.is_some() {
        changed |= s.reload_owner(false);
        tx.wake_after(OWNER_EVERY);
    }
    changed |= s.gh.poll(&s.agents, s.focused, s.waker.as_ref());
    changed |= s.plans.poll(s.focused, s.waker.as_ref(), tx);
    changed |= s.host.poll(s.focused, tx);
    changed |= s.context.poll(s.selected_agent.as_deref(), s.focused, tx);
    changed |= s.session.poll(s.selected_agent.as_deref(), s.focused, s.waker.as_ref(), tx);
    changed |= s.update.poll(s.focused, s.waker.as_ref(), tx);
    s.sync_focus();
    s.sync_outline();
    // Shells that exited (`exit`, Ctrl+D) close their tab, as in contr.
    let exited: Vec<u64> = s.tabs.iter().filter(|t| t.session.exited().load(Ordering::Acquire)).map(|t| t.session.id()).collect();
    for id in exited {
        s.close_tab(id);
        changed = true;
    }
    for t in &s.tabs {
        if t.session.dirty().swap(false, Ordering::AcqRel) { changed = true; }
    }
    let now = now_ms();
    let statuses: Vec<(u64, Status)> = s
        .tabs
        .iter()
        .map(|t| {
            let quiet = now.saturating_sub(t.session.last_output().load(Ordering::Relaxed));
            (t.session.id(), if quiet < WORKING_MS { Status::Working } else { Status::Idle })
        })
        .collect();
    if statuses.iter().any(|(_, st)| *st == Status::Working) {
        // Look again when the newest output turns quiet, to drop the working dot.
        tx.wake_after(Duration::from_millis(500));
    }
    // A terminal of the selected project going from working to idle may have changed files.
    let finished = s.statuses.iter().any(|(id, was)| {
        *was == Status::Working
            && statuses.iter().any(|(i, st)| i == id && *st == Status::Idle)
            && s.tabs.iter().any(|t| t.session.id() == *id && t.project == s.selected_project)
    });
    if statuses != s.statuses {
        s.statuses = statuses;
        changed = true;
    }
    changed | poll_diff(s, finished, tx)
}

/// Take finished git runs and start the next one: at once for a new project, else at most every 3 s.
fn poll_diff(s: &mut App, finished: bool, tx: &mut Tx) -> bool {
    let mut changed = false;
    let path = s.selected_path();
    while let Ok((from, scan)) = s.diff_rx.try_recv() {
        s.diff_inflight = false;
        if Some(&from) == path.as_ref() {
            s.apply_scan(scan);
            s.diff_for = Some(from);
            changed = true;
        }
    }
    let new_project = s.diff_for != path;
    if new_project {
        s.diff = None;
        s.tree = files::empty();
        s.diff_for = path;
        s.diff_stale = true;
        changed = true;
    }
    s.diff_stale |= finished;
    if s.diff_stale && !s.diff_inflight {
        match s.diff_last.map(|t| DIFF_EVERY.saturating_sub(t.elapsed())) {
            Some(wait) if !new_project && !wait.is_zero() => tx.wake_after(wait),
            _ => s.spawn_diff(),
        }
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
    if notch::key(s, ev) {
        return true;
    }
    if is_shortcut(ev, 'j') {
        s.toggle_card();
        return true;
    }
    if s.find_key(ev, tx) {
        return true;
    }
    if is_shortcut(ev, 't') {
        s.open_tab(s.selected_project);
        return true;
    }
    if is_shortcut(ev, 'w') {
        if let Some(id) = s.active { s.close_tab(id); }
        return true;
    }
    if is_shortcut(ev, 'b') {
        s.toggle_files();
        return true;
    }
    if is_shortcut(ev, ',') {
        s.settings_open = true;
        return true;
    }
    if is_shortcut(ev, 'd') {
        s.toggle_split();
        return true;
    }
    // ⌘[ and ⌘] move the focus left and right; with Shift held (off macOS) the keys read as { and }.
    if is_shortcut(ev, '[') || is_shortcut(ev, '{') {
        s.focus_pane(0);
        return true;
    }
    if is_shortcut(ev, ']') || is_shortcut(ev, '}') {
        s.focus_pane(1);
        return true;
    }
    if is_shortcut(ev, 'c') {
        if let Some(text) = select::copied(s) { tx.copy(text); }
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
        Key::PageUp if ev.mods.shift => { scroll_active(s, s.pane_size[s.focus_idx()].rows as i32 - 1); return true; }
        Key::PageDown if ev.mods.shift => { scroll_active(s, -(s.pane_size[s.focus_idx()].rows as i32 - 1)); return true; }
        _ => {}
    }
    // While the bar's or the projects modal's input has the focus, typing belongs to it.
    if s.find.open && s.find.edit.focused || s.pm.open { return false; }
    let Some(t) = s.active_tab() else { return false };
    let mode = *t.session.term().lock().unwrap().mode();
    match keys::bytes(ev, mode) {
        Some(b) => { select::clear(s, t.session.id()); t.session.write(&b); true }
        None => false,
    }
}

fn scroll_active(s: &mut App, lines: i32) {
    if let Some(id) = s.active { scroll_tab(s, id, lines); }
}

fn scroll_tab(s: &App, id: u64, lines: i32) {
    if let Some(t) = s.tabs.iter().find(|t| t.session.id() == id) { t.session.scroll(lines); }
}

// ───────────────────────────── view ─────────────────────────────

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let kit = Kit::neutral(s.appearance.is_dark(cx.system_dark()));
    let k = &kit;
    let mut layer = Vec::new();
    let bar = title_bar(k, s, cx, &mut layer);
    if s.settings_open {
        dialog::build(k, s, &mut layer);
    }
    if s.pm.open {
        projects_modal::build(k, s, &mut layer);
    }
    notch::card(k, s, &mut layer);
    update::overlays(k, s, &mut layer);
    outline::card(k, s, &mut layer);
    shell::window(k, bar, sidebar::view(k, s, &mut layer), terminal_panel(k, s), s.show_files.then(|| files_panel(k, s)), layer)
        .on_window_focus(|s: &mut App, focused, _tx| {
            s.focused = focused;
            // Coming back: the file may have changed meanwhile.
            if focused { s.reload_owner(false); s.update.on_focus(); }
        })
}

fn label(t: impl Into<String>, style: rune::tok::TextStyle, color: Color) -> El {
    text(t.into()).font(style).color(color)
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

/// Typed into the shell by the `ansi` scene: the 16 colours as text, dim, bold, inverse, black text.
const ANSI_DEMO: &str = concat!(
    "clear; for i in 0 1 2 3 4 5 6 7; do printf \"\\033[3${i}m color-$i \\033[9${i}m bright-$i \\033[0m\"; done; echo; ",
    "printf '\\033[2mdim text\\033[0m \\033[1mbold\\033[0m \\033[7minverse\\033[0m \\033[30mblack text\\033[0m \\033[90mbright black\\033[0m\\n'\r"
);

/// Typed into the shell by the `find` scenes: four lines hold "error" in different cases (the word is
/// split in the command, so its echo does not match), the others do not.
const FIND_DEMO: &str = concat!(
    "clear; printf '%s%s: unable to resolve import\\n%s%s: expected semicolon\\nwarning: unused variable\\n",
    "%s%s: type mismatch\\n%s%s: file not found\\nall done\\n' err or ERR OR Err or err or\r"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_size_floors_and_keeps_a_minimum() {
        assert_eq!(grid_size(CELL_W * 100.0 + 3.0, LINE_H * 30.0 + 7.0), Size { cols: 100, rows: 30 });
        assert_eq!(grid_size(0.0, 0.0), Size { cols: 2, rows: 1 });
    }
}

fn main() {
    if let Some(code) = cli::run(std::env::args_os()) { std::process::exit(code) }
    let spec = AppSpec::<App> {
        min_size: Some((720.0, 420.0)),
        fonts: FONTS,
        families: FAMILIES,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: &[
            ("three-tabs", |s| {
                s.open_tab(0);
                s.open_tab(0);
                s.open_tab(0);
                s.refresh_diff_now();
            }),
            // The agents of the fake core's fixture under their projects (`CONTR_HC_FAKE`).
            ("agents", |s| {
                s.refresh_diff_now();
            }),
            // The settings dialog open, in dark.
            ("settings", |s| {
                s.appearance = Appearance::Dark;
                s.settings_open = true;
                s.open_tab(0);
                s.refresh_diff_now();
            }),
            // The projects modal open, in dark.
            ("projects", |s| {
                s.appearance = Appearance::Dark;
                s.open_projects_modal();
                s.open_tab(0);
                s.refresh_diff_now();
            }),
            // The whole app in the light theme, the dialog closed.
            ("light", |s| {
                s.appearance = Appearance::Light;
                s.open_tab(0);
                s.open_tab(0);
                s.open_tab(0);
                s.refresh_diff_now();
            }),
            // One shell that prints the ANSI colours, to check the palette on the panel.
            ("ansi", |s| {
                s.open_tab(0);
                if let Some(t) = s.active_tab() { t.session.write(ANSI_DEMO.as_bytes()); }
                s.refresh_diff_now();
            }),
            // A split: the ANSI colours in the left pane, the find text in the right one (focused).
            ("split", |s| {
                s.open_tab(0);
                if let Some(t) = s.active_tab() { t.session.write(ANSI_DEMO.as_bytes()); }
                s.toggle_split();
                if let Some(t) = s.active_tab() { t.session.write(FIND_DEMO.as_bytes()); }
                s.refresh_diff_now();
            }),
            // The questions card open over a shell (`CONTR_OWNER_FILE`, and `CONTR_HC_FAKE` for blocked agents).
            ("questions", |s| {
                s.open_tab(0);
                s.refresh_diff_now();
                s.card.open = true;
                // The tab of the first question, after the blocked agent of the fake core's fixture.
                s.card.tab = 1;
            }),
            // Three shells and the update check run at once (`CONTR_UPDATE_URL`): the pill and the toast show for a newer release.
            ("update", |s| {
                s.open_tab(0);
                s.open_tab(0);
                s.open_tab(0);
                s.refresh_diff_now();
                s.update.check_now();
            }),
            // One shell with known text, the find bar closed (the tests type the query).
            ("find-text", |s| {
                s.open_tab(0);
                if let Some(t) = s.active_tab() { t.session.write(FIND_DEMO.as_bytes()); }
                s.refresh_diff_now();
            }),
            // The same, with the find bar open on "error": every match filled, the first one stronger.
            ("find", |s| {
                s.open_tab(0);
                if let Some(t) = s.active_tab() { t.session.write(FIND_DEMO.as_bytes()); }
                s.refresh_diff_now();
                s.open_find();
                s.find.edit.replace_text("error");
                s.find.edit.focused = true;
            }),
        ],
        ..AppSpec::new("contr", (1280.0, 800.0), init, view)
    };
    // The 38 px title bar is ours; the traffic lights sit in its left area (first circle at x 18, centred).
    rune::App::new(spec).titlebar(rune::Titlebar::new(TITLEBAR_HEIGHT).traffic_lights(17.0, 12.0)).run();
}
