//! The files panel's data (K6): the selected project's file tree with a change state per file.
//! [`scan`] runs off the UI thread; it lists the files (`.gitignore` respected in a repo, a capped
//! directory walk otherwise), reads the changes from [`crate::diff`] and builds the tree model.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use rune::{Icon, El, col, row, spacer, text};
use rune_kit::{ControlSize, FileNode, Progress, FileTree, IconButton, IconButtonVariant, Kit, State, TreeItem, TreeModel, TreeStyle, Tree};

use crate::{App, label, session_panel, shell, usage};
use crate::theme::{FILE_ROW_HEIGHT, ROW_SMALL};
use crate::diff::{self, ChangeKind, Changed, DiffSummary};

/// Outside a repo the walk stops after this many files.
const WALK_CAP: usize = 20_000;
/// Folders a plain walk does not enter.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// What differs from `HEAD` for one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Modified,
    New,
    Deleted,
}

/// A file's change state and line counts (the tree shows them as `+a −r`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub added: u32,
    pub removed: u32,
    pub kind: Kind,
}

impl Change {
    pub fn is_changed(&self) -> bool {
        self.kind != Kind::Plain
    }
}

pub type Model = TreeModel<FileNode<Change>>;

/// One scan of a project: the tree and the diff summary, computed from the same change list.
pub struct Scan {
    pub model: Model,
    /// `None` when the folder is not a repo.
    pub summary: Option<DiffSummary>,
}

/// An empty tree (before the first scan finishes).
pub fn empty() -> Model {
    build(Vec::new(), &[])
}

/// The files of the project at `dir`, as paths from `dir` with `/` separators, unsorted.
pub fn list_files(dir: &Path, is_repo: bool) -> Vec<String> {
    if is_repo && let Some(out) = diff::run_git(dir, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"]) {
        return out.split(|b| *b == 0).filter(|n| !n.is_empty()).map(|n| String::from_utf8_lossy(n).into_owned()).collect();
    }
    walk(dir)
}

/// A plain directory walk: no symlinks, no [`SKIP_DIRS`], at most [`WALK_CAP`] files.
pub fn walk(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(ty) = entry.file_type() else { continue };
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if ty.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((entry.path(), rel));
                }
            } else if ty.is_file() {
                if files.len() >= WALK_CAP {
                    return files;
                }
                files.push(rel);
            }
        }
    }
    files
}

fn clamp(n: u64) -> u32 {
    n.min(u32::MAX as u64) as u32
}

/// The tree of `paths` plus every changed path (a deleted file may be missing from the list),
/// folders first and sorted. Folders that hold a change start open; the others closed.
pub fn build(paths: Vec<String>, changes: &[Changed]) -> Model {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries: Vec<(String, Change)> = Vec::with_capacity(paths.len());
    for c in changes {
        let kind = match c.kind {
            ChangeKind::Modified => Kind::Modified,
            ChangeKind::New => Kind::New,
            ChangeKind::Deleted => Kind::Deleted,
        };
        if seen.insert(c.path.clone()) {
            entries.push((c.path.clone(), Change { added: clamp(c.added), removed: clamp(c.removed), kind }));
        }
    }
    for p in paths {
        if seen.insert(p.clone()) {
            entries.push((p, Change { added: 0, removed: 0, kind: Kind::Plain }));
        }
    }
    let changed: Vec<String> = entries.iter().filter(|(_, c)| c.is_changed()).map(|(p, _)| p.clone()).collect();
    let mut model = FileTree::new().compact_folders(false).key("tree").build(entries);
    for path in changed {
        let mut end = 0;
        while let Some(i) = path[end..].find('/') {
            end += i;
            model.set_expanded(&path[..end], true);
            end += 1;
        }
    }
    model
}

/// Scan the project at `dir`: files, changes, tree and summary.
pub fn scan(dir: &Path) -> Scan {
    let changes = diff::changes(dir);
    let paths = list_files(dir, changes.is_some());
    let model = build(paths, changes.as_deref().unwrap_or(&[]));
    Scan { model, summary: changes.as_deref().map(diff::summary_of) }
}

/// Keep what the user did to `old` (open and closed folders, the selected row) in `new`, a fresh
/// scan of the same project. Folders that are new in `new` keep their initial state.
pub fn carry_state(new: &mut Model, mut old: Model) {
    old.clear_filter();
    let mut stack: Vec<Option<String>> = vec![None];
    while let Some(parent) = stack.pop() {
        let ids: Vec<String> = new.children(parent.as_deref()).map(str::to_owned).collect();
        for id in ids {
            if new.get(&id).is_some_and(|n| n.dir) {
                if old.contains(&id) {
                    new.set_expanded(&id, old.is_expanded(&id));
                }
                stack.push(Some(id));
            }
        }
    }
    if let Some(c) = old.cursor()
        && new.contains(c)
    {
        let c = c.to_owned();
        new.select(&c);
    }
}

/// Show only changed files and their folders, or everything.
pub fn set_changed_only(model: &mut Model, on: bool) {
    if on {
        model.filter(|n| n.data.is_some_and(|c| c.is_changed()));
    } else {
        model.clear_filter();
    }
}

/// The program that opens a file with its default app.
pub fn opener() -> &'static str {
    if cfg!(target_os = "macos") { "open" } else { "xdg-open" }
}

/// Open `path` with the system default app; errors are ignored (nothing to show them in).
pub fn open_with_default_app(path: &Path) {
    if let Ok(mut child) = Command::new(opener()).arg(path).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(path: &str, added: u64, removed: u64, kind: ChangeKind) -> Changed {
        Changed { path: path.into(), added, removed, kind }
    }

    fn rows(m: &Model) -> Vec<String> {
        (0..m.visible_len()).filter_map(|i| m.visible_id(i)).map(str::to_owned).collect()
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn folders_first_then_files_sorted() {
        let mut m = build(paths(&["b.txt", "a.txt", "z/x.rs", "m/y.rs", "A2.txt"]), &[]);
        m.expand_all();
        assert_eq!(rows(&m), ["m", "m/y.rs", "z", "z/x.rs", "a.txt", "A2.txt", "b.txt"]);
    }

    #[test]
    fn folders_without_changes_start_closed() {
        let m = build(paths(&["a/b/c.rs", "d/e.rs"]), &[]);
        assert_eq!(rows(&m), ["a", "d"]);
    }

    #[test]
    fn folders_with_changes_start_open_all_the_way_up() {
        let m = build(paths(&["a/b/c.rs", "a/b/d.rs", "z/q.rs", "k/l.rs"]), &[ch("a/b/c.rs", 2, 1, ChangeKind::Modified)]);
        assert_eq!(rows(&m), ["a", "a/b", "a/b/c.rs", "a/b/d.rs", "k", "z"]);
    }

    #[test]
    fn deleted_files_missing_from_the_list_are_added() {
        let m = build(paths(&["keep.txt"]), &[ch("gone/old.txt", 0, 4, ChangeKind::Deleted)]);
        assert_eq!(rows(&m), ["gone", "gone/old.txt", "keep.txt"]);
        let n = m.get("gone/old.txt").unwrap();
        assert_eq!(n.data, Some(Change { added: 0, removed: 4, kind: Kind::Deleted }));
    }

    #[test]
    fn a_changed_file_in_the_list_keeps_its_change_once() {
        let m = build(paths(&["a.txt", "b.txt"]), &[ch("a.txt", 3, 0, ChangeKind::New)]);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("a.txt").unwrap().data.unwrap().kind, Kind::New);
        assert_eq!(m.get("b.txt").unwrap().data.unwrap().kind, Kind::Plain);
    }

    #[test]
    fn changed_only_keeps_changed_files_and_their_folders() {
        let mut m = build(paths(&["a/b.rs", "a/c.rs", "d/e.rs", "f.txt"]), &[ch("a/b.rs", 1, 0, ChangeKind::Modified), ch("f.txt", 2, 0, ChangeKind::New)]);
        set_changed_only(&mut m, true);
        assert_eq!(rows(&m), ["a", "a/b.rs", "f.txt"]);
        set_changed_only(&mut m, false);
        assert_eq!(rows(&m), ["a", "a/b.rs", "a/c.rs", "d", "f.txt"]);
    }

    #[test]
    fn carry_state_keeps_open_folders_and_selection() {
        let mut old = build(paths(&["a/x.rs", "b/y.rs"]), &[ch("a/x.rs", 1, 0, ChangeKind::Modified)]);
        old.set_expanded("a", false);
        old.set_expanded("b", true);
        old.select("b/y.rs");
        let mut new = build(paths(&["a/x.rs", "b/y.rs", "c/z.rs"]), &[ch("a/x.rs", 1, 0, ChangeKind::Modified), ch("c/z.rs", 1, 0, ChangeKind::New)]);
        carry_state(&mut new, old);
        assert!(!new.is_expanded("a"));
        assert!(new.is_expanded("b"));
        assert!(new.is_expanded("c"), "a new folder keeps its initial state");
        assert_eq!(new.cursor(), Some("b/y.rs"));
    }

    #[test]
    fn walk_skips_heavy_folders_and_lists_files() {
        let root = std::env::temp_dir().join(format!("contr-rune-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["src", "target/debug", "node_modules/x", ".git/objects"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["src/main.rs", "README.md", "target/debug/a", "node_modules/x/i.js", ".git/HEAD"] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let mut got = walk(&root);
        got.sort();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(got, ["README.md", "src/main.rs"]);
    }
}

// The files card (view) ─────────────

static FILTER: Icon = Icon {
    name: "contr-filter",
    view: 16.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 4h11M4.5 8h7M6.5 12h3"/></g>"#,
};

/// The files card: "Files" and the changed-only toggle, then the session panel (C4), the tree, then the selected
/// agent's context meter (H13) when it has a context file.
pub(crate) fn files_panel(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let label_ = "Show changed files only";
    let toggle = IconButton::new(&FILTER)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Large)
        .label(label_)
        .help(label_)
        .state(if s.changed_only { State::Pressed } else { State::Rest })
        .on_press(|s: &mut App| s.toggle_changed_only())
        .build(k)
        .key("files-changed-only");
    let head = row()
        .key("files/head")
        .w_full()
        .h(28.0)
        .pl(10.0)
        .pr(2.0)
        .center_y()
        .child(text("Files").key("files/title").font(k.ty.small_medium).color(c.text_faint))
        .child(spacer())
        .child(toggle);
    let body = if s.changed_only && s.tree.visible_len() == 0 {
        col().grow(1.0).min_h(0.0).w_full().pt(12.0).px(10.0).child(label("No changed files", k.ty.text_small, c.text_faint))
    } else {
        file_tree(k, s)
    };
    let mut panel = shell::files_panel(k).padding(10.0).child(head);
    if let Some(session) = session_panel::view(k, s) {
        panel = panel.child(session);
    }
    let panel = panel.child(body);
    match s.context.context.as_ref() {
        Some(ctx) => panel.child(context_meter(k, ctx)),
        None => panel,
    }
}

/// "Context" with the value ("90k of 200k  45%") at the right, and a bar under it.
fn context_meter(k: &Kit, ctx: &usage::Context) -> El {
    let c = &k.c;
    col()
        .key("meter/context")
        .w_full()
        .gap(6.0)
        .pt(8.0)
        .px(10.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(text("Context").key("label").font(k.ty.text_xsmall).color(c.text_faint))
                .child(spacer())
                .child(text(ctx.label()).key("value").font(k.ty.mono).color(c.text_muted)),
        )
        .child(Progress::new(Some(ctx.percent() as f32 / 100.0)).build(k).key("bar"))
}

fn file_tree(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let style = {
        let mut st = TreeStyle::from_kit(k).label(ROW_SMALL, c.text_muted, c.text_strong).hover(c.hover);
        st.row_height = FILE_ROW_HEIGHT;
        st.indent = 21.0;
        st
    };
    let (rest, strong, faint) = (c.text_muted, c.text_strong, c.text_faint);
    let root = s.selected_path();
    Tree::new(|s: &App| &s.tree, |s: &mut App| &mut s.tree, |n: &FileNode<Change>| n.name.as_str())
        .name("Files")
        .style(style)
        .row(move |n, r| {
            let item = TreeItem::file(n, r);
            match n.data {
                Some(Change { added, removed, kind }) => {
                    let item = match kind {
                        Kind::Modified | Kind::New => item.label_color(strong),
                        Kind::Deleted => item.label_color(faint).icon_color(faint),
                        Kind::Plain => item.label_color(rest),
                    };
                    if added + removed > 0 { item.changes(added, removed) } else { item }
                }
                None => item,
            }
        })
        .on_open(move |_s: &mut App, id: &str, _tx| {
            if let Some(root) = &root {
                open_with_default_app(&root.join(id));
            }
        })
        .build(s, k)
        .grow(1.0)
        .min_h(0.0)
}
