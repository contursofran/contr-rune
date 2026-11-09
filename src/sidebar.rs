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
//! `term-<id>`, `server-this-mac`, `settings`.

use std::path::Path;
use std::sync::atomic::Ordering;

use rune::{Color, Cycle, El, FileDialogOptions, Icon, LoopSpec, el, icon, row, spacer, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarRow};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit};

use crate::hc::model::{self, Agent, AgentState, Line};
use crate::session::now_ms;
use crate::theme;
use crate::{App, PLUS, Project, Status, TERM, shell};

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
pub fn agent_glyph(k: &Kit, a: &Agent) -> El {
    let word = match a.state {
        AgentState::NeedsYou { .. } => "needs you",
        AgentState::Working => "working",
        AgentState::Waiting => "waiting",
        AgentState::Idle | AgentState::Ended => "idle",
    };
    if a.is_orchestrator() {
        let mark = icon(&CLAUDE, 16.0).key("glyph").label(word.to_owned());
        return match a.state {
            AgentState::NeedsYou { .. } => mark.color(k.c.danger.icon),
            AgentState::Working | AgentState::Waiting => mark.color(theme::claude_color()).looping(LoopSpec::rotation(2.4)),
            AgentState::Idle | AgentState::Ended => mark.color(theme::claude_color()).opacity(0.45),
        };
    }
    match a.state {
        AgentState::NeedsYou { .. } => braille(k.c.danger.icon, word),
        AgentState::Working => braille(k.c.text_base, word),
        AgentState::Waiting => text("⠿").key("glyph").label(word.to_owned()).font(theme::GLYPH).color(k.c.text_base).looping(LoopSpec::opacity(1.6, 0.0, 0.25, 1.0)),
        AgentState::Idle | AgentState::Ended => text("⠶").key("glyph").label(word.to_owned()).font(theme::GLYPH).color(k.c.text_faint),
    }
}

fn agent_row(k: &Kit, s: &App, line: &Line) -> El {
    let a = line.agent;
    let address = a.address.clone();
    let row = SidebarRow::new(a.display_title().to_owned())
        .avatar(agent_glyph(k, a))
        .subtitle(model::meta(line))
        .subtitle_style(theme::META)
        .title_style(theme::BODY)
        .detail(fmt_age(a.age_ms / 1000))
        .height(if line.depth <= 1 { ORCHESTRATOR_ROW_HEIGHT } else { WORKER_ROW_HEIGHT })
        .depth(line.depth)
        .selected(s.selected_agent.as_deref() == Some(a.address.as_str()))
        .on_select(move |s: &mut App, _tx| s.select_agent(address.clone()))
        .build(k)
        .key(format!("agent/{}", a.address));
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

pub fn view(k: &Kit, s: &App) -> El {
    let add_project = icon_button(&PLUS, "Open a folder")
        .on_press_tx(|_: &mut App, tx| {
            let options = FileDialogOptions { title: "Open a folder".into(), ..Default::default() };
            tx.open_folder(options, |s: &mut App, result, _| match result {
                Ok(Some(path)) => s.add_project(path),
                Ok(None) => {}
                Err(e) => s.error = Some(format!("couldn't open a folder: {e}")),
            });
        })
        .build(k)
        .key("add-project");
    let mut list = Sidebar::new().label("Projects").inset(0.0).row_gap(2.0).header(section_head(k, "Projects", Some(add_project)));
    for (i, p) in s.projects.iter().enumerate() {
        if i > 0 {
            list = list.child(el().h(10.0));
        }
        list = list.child(project_row(k, s, i, p));
        for line in model::lines(&s.agents, i) {
            list = list.child(agent_row(k, s, &line));
        }
        for t in s.project_tabs(i) {
            list = list.child(terminal_row(k, s, t));
        }
    }
    let this_mac = SidebarRow::new("this Mac").icon(&LAPTOP).detail("local").height(theme::ROW_HEIGHT).title_style(theme::BODY).selected(true).build(k).key("server-this-mac");
    let gear = icon_button(&GEAR, "Settings").on_press(|s: &mut App| s.settings_open = true).build(k).key("settings");
    if let Some(e) = &s.error {
        list = list.footer(text(e.clone()).font(k.ty.text_xsmall).color(k.c.danger.icon).key("error").px(10.0).pb(6.0));
    }
    let list = list
        .footer(section_head(k, "Servers", None))
        .footer(this_mac)
        .footer(row().w_full().h(1.0).fill(k.c.border_muted).mt(6.0))
        .footer(row().w_full().pt(8.0).pr(2.0).child(spacer()).child(gear))
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
