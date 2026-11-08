//! The sidebar (K4): rune-kit's `Sidebar` and `SidebarRow`. A "Projects" head with `+` (open a
//! folder), each project with its terminals under it, then "Servers" and the settings gear.
//!
//! Element keys: `add-project`, `project-<i>`, `add-<i>` (the project's hover `+`), `term-<id>`,
//! `server-this-mac`, `settings`.

use std::path::Path;
use std::sync::atomic::Ordering;

use rune::{El, FileDialogOptions, Icon, el, icon, row, spacer, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarRow};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit};

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
static LAPTOP: Icon = Icon {
    name: "contr-laptop",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="10" height="7" rx="1.2"/><path d="M1.5 12.5h13"/></g>"#,
};

/// A terminal row is two lines (title, mono subtitle) under its project.
const TERMINAL_ROW_HEIGHT: f32 = 48.0;

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

fn terminal_row(k: &Kit, s: &App, t: &crate::Tab) -> El {
    let id = t.session.id;
    let age = now_ms().saturating_sub(t.session.last_output.load(Ordering::Relaxed)) / 1000;
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
    if s.status(id) == Status::Working {
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
