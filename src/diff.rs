//! The title bar's diff summary (K5) and the files panel's change list (K6): what changed in a
//! project's working tree against `HEAD`. Tracked changes come from `git diff` (numstat for the
//! lines, name-status for new/deleted); untracked files (not ignored) are new and count all their
//! lines as added. The summary is the sum of the change list, so the title bar and the files tree
//! always agree. Run it off the UI thread.

use std::path::Path;
use std::process::Command;

/// Untracked files larger than this are counted as one file with no lines.
const MAX_UNTRACKED_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DiffSummary {
    pub added: u64,
    pub removed: u64,
    pub files: u64,
}

impl DiffSummary {
    /// `+120  −126  9 files` (`1 file` for one). `None` when nothing changed.
    pub fn text(&self) -> Option<String> {
        if self.files == 0 {
            return None;
        }
        let noun = if self.files == 1 { "file" } else { "files" };
        Some(format!("+{}  \u{2212}{}  {} {noun}", self.added, self.removed, self.files))
    }
}

/// How a path differs from `HEAD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    New,
    Deleted,
}

/// One changed file: its path from the repo root (`/` separated) and its line counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub kind: ChangeKind,
}

/// The summary of a list of changed files: one file each, lines summed.
pub fn summary_of(changes: &[Changed]) -> DiffSummary {
    DiffSummary {
        added: changes.iter().map(|c| c.added).sum(),
        removed: changes.iter().map(|c| c.removed).sum(),
        files: changes.len() as u64,
    }
}

/// Parse `git diff --numstat -z --no-renames` output into `(path, added, removed)`. A binary file
/// (`-\t-\tpath`) has no lines.
pub fn parse_numstat(out: &[u8]) -> Vec<(String, u64, u64)> {
    let mut list = Vec::new();
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let mut parts = rec.trim_start_matches('\n').splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        list.push((path.to_owned(), a.parse().unwrap_or(0), r.parse().unwrap_or(0)));
    }
    list
}

/// Parse `git diff --name-status -z --no-renames` output into `(path, kind)`.
pub fn parse_name_status(out: &[u8]) -> Vec<(String, ChangeKind)> {
    let mut toks = out.split(|b| *b == 0).filter(|r| !r.is_empty()).map(|t| String::from_utf8_lossy(t).into_owned());
    let mut list = Vec::new();
    while let (Some(status), Some(path)) = (toks.next(), toks.next()) {
        let kind = match status.chars().next() {
            Some('A') => ChangeKind::New,
            Some('D') => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        };
        list.push((path, kind));
    }
    list
}

/// The lines of a file's bytes: 0 for binary (a NUL byte) or empty; a last line without `\n` counts.
pub fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() || bytes.contains(&0) {
        return 0;
    }
    let n = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if bytes.ends_with(b"\n") { n } else { n + 1 }
}

pub fn run_git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Whether `dir` is inside a work tree (and the `git` binary exists).
pub fn is_repo(dir: &Path) -> bool {
    run_git(dir, &["rev-parse", "--is-inside-work-tree"]).is_some()
}

/// The changed files of the repo at `dir`, sorted by path; `None` when `dir` is not a work tree or
/// the `git` binary is missing.
pub fn changes(dir: &Path) -> Option<Vec<Changed>> {
    if !is_repo(dir) {
        return None;
    }
    // No commits yet: `HEAD` does not exist, so there are no tracked changes to count.
    let kinds = run_git(dir, &["diff", "--name-status", "-z", "--no-renames", "HEAD"]).map(|o| parse_name_status(&o)).unwrap_or_default();
    let mut list: Vec<Changed> = run_git(dir, &["diff", "--numstat", "-z", "--no-renames", "HEAD"])
        .map(|o| parse_numstat(&o))
        .unwrap_or_default()
        .into_iter()
        .map(|(path, added, removed)| {
            let kind = kinds.iter().find(|(p, _)| *p == path).map_or(ChangeKind::Modified, |(_, k)| *k);
            Changed { path, added, removed, kind }
        })
        .collect();
    if let Some(untracked) = run_git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        for name in untracked.split(|b| *b == 0).filter(|n| !n.is_empty()) {
            let path = String::from_utf8_lossy(name).into_owned();
            let full = dir.join(&path);
            let added = match std::fs::metadata(&full) {
                Ok(m) if m.is_file() && m.len() <= MAX_UNTRACKED_BYTES => std::fs::read(&full).map(|b| count_lines(&b)).unwrap_or(0),
                _ => 0,
            };
            list.push(Changed { path, added, removed: 0, kind: ChangeKind::New });
        }
    }
    list.sort_by(|a, b| a.path.cmp(&b.path));
    list.dedup_by(|a, b| a.path == b.path);
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(path: &str, added: u64, removed: u64, kind: ChangeKind) -> Changed {
        Changed { path: path.into(), added, removed, kind }
    }

    #[test]
    fn numstat_reads_paths_and_counts() {
        let l = parse_numstat(b"3\t1\tsrc/a.rs\x0010\t0\tb c.txt\0");
        assert_eq!(l, vec![("src/a.rs".to_owned(), 3, 1), ("b c.txt".to_owned(), 10, 0)]);
    }

    #[test]
    fn numstat_binary_has_no_lines() {
        assert_eq!(parse_numstat(b"-\t-\timg.png\0"), vec![("img.png".to_owned(), 0, 0)]);
    }

    #[test]
    fn numstat_ignores_blank_and_malformed_records() {
        assert!(parse_numstat(b"").is_empty());
        assert!(parse_numstat(b"\0not numstat\0").is_empty());
    }

    #[test]
    fn name_status_maps_kinds() {
        let l = parse_name_status(b"M\0a.rs\0A\0b.rs\0D\0c/d.rs\0T\0e\0");
        assert_eq!(
            l,
            vec![
                ("a.rs".to_owned(), ChangeKind::Modified),
                ("b.rs".to_owned(), ChangeKind::New),
                ("c/d.rs".to_owned(), ChangeKind::Deleted),
                ("e".to_owned(), ChangeKind::Modified),
            ]
        );
    }

    #[test]
    fn summary_sums_changes() {
        let s = summary_of(&[ch("a", 3, 1, ChangeKind::Modified), ch("b", 10, 0, ChangeKind::New), ch("c", 0, 2, ChangeKind::Deleted)]);
        assert_eq!(s, DiffSummary { added: 13, removed: 3, files: 3 });
        assert_eq!(summary_of(&[]), DiffSummary::default());
    }

    #[test]
    fn text_formats_and_hides() {
        assert_eq!(DiffSummary { added: 120, removed: 126, files: 9 }.text().as_deref(), Some("+120  \u{2212}126  9 files"));
        assert_eq!(DiffSummary { added: 1, removed: 0, files: 1 }.text().as_deref(), Some("+1  \u{2212}0  1 file"));
        assert_eq!(DiffSummary::default().text(), None);
    }

    #[test]
    fn lines_of_text_and_binary() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b"\0\n\n"), 0);
    }
}
