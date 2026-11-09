//! The projects modal (C6, Go "Projects modal"): from "Open projects…" in the Projects `+` menu.
//! A filter, the open projects with a grip to drag them into the sidebar order and `close`
//! (hides the project in the sidebar only: its terminals and agents keep running), the `recent`
//! (hidden) projects with `open`, `+ open folder…` and Done. Esc, `esc` and Done close it. There
//! is no clone.
//!
//! Hidden projects stay in `App::projects` with `hidden` set, so every project index (tabs, agents,
//! `project-<i>`) stays valid; only a reorder renumbers them (`move_project`). Keys:
//! `projects-modal`, `projects-modal/filter`, `projects-modal/open/<i>`, `projects-modal/grip/<i>`,
//! `projects-modal/close/<i>`, `projects-modal/recent/<i>` (and `.../open`), `projects-modal/folder`,
//! `projects-modal/done`; `<i>` is the project's index (`project-<i>`), or the place in `recent`.

use std::path::{Path, PathBuf};

use rune::{Cursor, Drag, El, Key, KeyEvent, Phase, TextEdit, Tx, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextInput};

use crate::hc::model;
use crate::hc::owner::rfc3339_now;
use crate::projects::{MAX_RECENT, Recent};
use crate::{App, Project, sidebar};

const WIDTH: f32 = 580.0;
const TOP: f32 = 110.0;
const RADIUS: f32 = crate::theme::RADIUS_12;
/// Height of one project row; a drag of the grip moves a row by this much per place.
const ROW_H: f32 = 52.0;

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub filter: TextEdit,
}

impl App {
    pub fn open_projects_modal(&mut self) {
        self.pm.open = true;
        self.pm.filter = TextEdit::new("");
        self.pm.filter.focused = true;
    }

    fn close_projects_modal(&mut self) {
        self.pm.open = false;
    }

    /// The indices of the projects in the sidebar, in order.
    fn shown_projects(&self) -> Vec<usize> {
        (0..self.projects.len()).filter(|i| !self.projects[*i].hidden).collect()
    }

    /// Take project `i` off the sidebar and put it first in `recent`. The selection moves to the
    /// first project left in the sidebar, if there is one.
    fn hide_project(&mut self, i: usize) {
        let Some(p) = self.projects.get_mut(i).filter(|p| !p.hidden) else { return };
        p.hidden = true;
        let path = p.path.clone();
        self.recent.retain(|r| r.path != path);
        self.recent.insert(0, Recent { path, at: rfc3339_now() });
        self.recent.truncate(MAX_RECENT);
        self.agent_menu.project = None;
        match self.shown_projects().first() {
            Some(next) if self.selected_project == i => self.select_project(*next),
            _ => self.save_projects(),
        }
    }

    /// Drag the `from`-th project of the sidebar to place `to`.
    fn move_project(&mut self, from: usize, to: usize) {
        let shown = self.shown_projects();
        if from == to || from >= shown.len() || to >= shown.len() {
            return;
        }
        let (old, target) = (shown[from], shown[to]);
        let mut order: Vec<usize> = (0..self.projects.len()).collect();
        order.remove(old);
        order.insert(target, old);
        let mut moved: Vec<Option<Project>> = std::mem::take(&mut self.projects).into_iter().map(Some).collect();
        self.projects = order.iter().map(|o| moved[*o].take().expect("each project once")).collect();
        // `order[new] = old`: what held an old index takes its place in `order`.
        for t in &mut self.tabs {
            t.project = order.iter().position(|o| *o == t.project).unwrap_or(t.project);
        }
        self.selected_project = order.iter().position(|o| *o == self.selected_project).unwrap_or(self.selected_project);
        self.agent_menu.project = None;
        // The agents get their project index from the paths: derive them again.
        self.refresh_agents();
        self.save_projects();
    }
}

/// The sidebar's order (D174): the projects with something open first, the empty ones after, each
/// group in the stored order. Hidden projects are left out. The indices stay the projects' own.
pub fn sidebar_order(s: &App) -> Vec<usize> {
    let (mut with, empty): (Vec<usize>, Vec<usize>) =
        s.shown_projects().into_iter().partition(|i| s.project_tabs(*i).next().is_some() || !model::lines(&s.agents, *i).is_empty());
    with.extend(empty);
    with
}

/// Whether `name` or `path` has the filter text in it (case-insensitive); an empty filter has all.
fn matches(filter: &str, name: &str, path: &Path) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || name.to_lowercase().contains(&f) || path.to_string_lossy().to_lowercase().contains(&f)
}

/// The path with the home folder as `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Two letters for the tile: the first letters of the first two words of the name, else its first two.
fn initials(name: &str) -> String {
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let letters: String = match (words.next(), words.next()) {
        (Some(a), Some(b)) => a.chars().take(1).chain(b.chars().take(1)).collect(),
        (Some(a), None) => a.chars().take(2).collect(),
        _ => "?".to_owned(),
    };
    letters.to_uppercase()
}

fn project_row(k: &Kit, name: &str, path: &Path, detail: Option<String>, dim: bool, grip: Option<El>, action: El) -> El {
    let c = &k.c;
    let name_color = if dim { c.text_muted } else { c.text_strong };
    let tile = el().w(28.0).h(28.0).radius(7.0).fill(c.layer_02).center().child(text(initials(name)).font(k.ty.small_medium).color(name_color));
    let sub = match detail {
        Some(d) => format!("{} · {d}", tilde(path)),
        None => tilde(path),
    };
    let lead = grip.unwrap_or_else(|| el().w(14.0));
    row()
        .w_full()
        .h(ROW_H)
        .px(10.0)
        .gap(12.0)
        .center_y()
        .radius(8.0)
        .child(lead)
        .child(tile)
        .child(col().grow(1.0).min_w(0.0).child(text(name.to_owned()).font(k.ty.ui_medium).color(name_color).truncate()).child(text(sub).font(k.ty.text_xsmall).color(c.text_faint).truncate()))
        .child(action)
}

/// A drag of the grip of the `pos`-th sidebar project moves it by whole rows; it lands on release.
fn grip(k: &Kit, i: usize, pos: usize) -> El {
    text("⋮⋮")
        .key(format!("projects-modal/grip/{i}"))
        .font(k.ty.mono)
        .color(k.c.text_faint)
        .w(14.0)
        .cursor(Cursor::Grab)
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if d.phase == Phase::Up {
                let to = (pos as f32 + (d.dy / ROW_H).round()).max(0.0) as usize;
                s.move_project(pos, to);
            }
        })
}

fn section_label(k: &Kit, t: &str) -> El {
    text(t.to_owned()).font(k.ty.text_xsmall).color(k.c.text_faint).px(18.0).pt(8.0).pb(4.0)
}

/// Push the scrim and the dialog onto the overlay `layer`.
pub fn build(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let close = |s: &mut App| s.close_projects_modal();
    let c = &k.c;
    let filter = s.pm.filter.text.clone();
    let shown = s.shown_projects();
    let dragging = filter.trim().is_empty();

    let head = row()
        .w_full()
        .px(18.0)
        .pt(16.0)
        .pb(12.0)
        .gap(8.0)
        .center_y()
        .child(text("Projects").font(k.ty.ui_medium.lh(20.0)).color(c.text_strong))
        .child(text(format!("{} open", shown.len())).key("projects-modal/count").font(k.ty.text_small).color(c.text_faint))
        .child(spacer())
        .child(Button::new("esc").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).on_press(close).build(k).key("projects-modal/esc"));

    let input = TextInput::new(&s.pm.filter)
        .placeholder("filter, or paste a path…")
        .label("Filter projects")
        .on_edit(|s: &mut App| &mut s.pm.filter)
        .on_key(|s: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
            if ev.key == Key::Escape {
                s.close_projects_modal();
                return true;
            }
            false
        })
        .on_submit(|s: &mut App, _tx| {
            // A pasted folder opens as a project.
            let path = PathBuf::from(s.pm.filter.text.trim());
            if path.is_absolute() && path.is_dir() {
                s.pm.filter = TextEdit::new("");
                s.add_project(path);
            }
        })
        .build(k)
        .key("projects-modal/filter");

    let mut open_list = col().w_full().px(8.0);
    let mut open_count = 0;
    for (pos, i) in shown.iter().copied().enumerate() {
        let p = &s.projects[i];
        if !matches(&filter, &p.name, &p.path) {
            continue;
        }
        open_count += 1;
        let agents = model::lines(&s.agents, i).len();
        let detail = (agents > 0).then(|| format!("{agents} agent{}", if agents == 1 { "" } else { "s" }));
        let hide = Button::new("close")
            .variant(ButtonVariant::Neutral)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.hide_project(i))
            .build(k)
            .key(format!("projects-modal/close/{i}"));
        let g = dragging.then(|| grip(k, i, pos));
        open_list = open_list.child(project_row(k, &p.name, &p.path, detail, false, g, hide).key(format!("projects-modal/open/{i}")));
    }
    if open_count == 0 {
        open_list = open_list.child(text("no open project matches").font(k.ty.text_small).color(c.text_faint).px(10.0).py(8.0));
    }

    let mut recent_list = col().w_full().px(8.0);
    let mut recent_count = 0;
    for (n, r) in s.recent.iter().enumerate() {
        let name = sidebar::folder_name(&r.path);
        let is_open = s.projects.iter().any(|p| p.path == r.path && !p.hidden);
        if is_open || !matches(&filter, &name, &r.path) {
            continue;
        }
        recent_count += 1;
        let path = r.path.clone();
        let open = Button::new("open")
            .variant(ButtonVariant::Contrast)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.add_project(path.clone()))
            .build(k)
            .key(format!("projects-modal/recent/{n}/open"));
        recent_list = recent_list.child(project_row(k, &name, &r.path, None, true, None, open).key(format!("projects-modal/recent/{n}")));
    }

    let folder = Button::new("+ open folder…")
        .variant(ButtonVariant::Ghost)
        .on_press_tx(|_s: &mut App, tx| sidebar::pick_folder(tx, "Open a folder", App::add_project))
        .build(k)
        .key("projects-modal/folder");
    let done = Button::new("Done").variant(ButtonVariant::Contrast).on_press(close).build(k).key("projects-modal/done");
    let footer = row().w_full().px(18.0).py(12.0).gap(14.0).center_y().child(folder).child(spacer()).child(done);

    let hairline = || el().w_full().h(1.0).fill(c.border_muted);
    let hint = if dragging { "open in contr · drag to reorder the sidebar" } else { "open in contr" };
    let mut body = col().w_full().pb(8.0).child(el().w_full().px(18.0).pb(8.0).child(input)).child(section_label(k, hint)).child(open_list);
    if recent_count > 0 {
        body = body.child(section_label(k, "recent")).child(recent_list);
    }
    let mut card = col()
        .key("projects-modal")
        .role("dialog")
        .label("Projects")
        .w(WIDTH)
        .radius(RADIUS)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .clip_rounded(RADIUS)
        .token_motion(k.motion.overlay)
        .child(head)
        .child(body)
        .child(hairline())
        .child(footer)
        .modal(true, close);
    for shadow in k.shadow_layers(&k.elevation.overlay) {
        card = card.box_shadow(shadow);
    }
    layer.push(el().key("projects-modal/scrim").absolute(0.0, 0.0).w_full().h_full().fill(c.scrim).token_motion(k.motion.overlay).a11y_hidden());
    layer.push(col().key("projects-modal/layer").absolute(0.0, 0.0).w_full().h_full().pt(TOP).items_center().child(card));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_matches_name_or_path_ignoring_case() {
        let p = Path::new("/work/Contr-Rune");
        for (f, want) in [("", true), ("  ", true), ("contr", true), ("WORK", true), ("rune", true), ("zzz", false)] {
            assert_eq!(matches(f, "Contr-Rune", p), want, "{f:?}");
        }
    }

    #[test]
    fn initials_take_two_words_or_two_letters() {
        for (name, want) in [("contr-rune", "CR"), ("alpha", "AL"), ("x", "X"), ("", "?"), ("a b c", "AB")] {
            assert_eq!(initials(name), want, "{name:?}");
        }
    }
}
