//! The title bar's diff summary (K5): what changed in a project's working tree against `HEAD`,
//! as `+<added> −<removed> <n> files`. Tracked changes come from `git diff --numstat HEAD`;
//! untracked files (not ignored) count all their lines as added. Run it off the UI thread.

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

/// Sum `git diff --numstat` output. A binary file (`-\t-\tpath`) counts as a file with no lines.
pub fn parse_numstat(out: &str) -> DiffSummary {
    let mut sum = DiffSummary::default();
    for line in out.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(r), Some(_path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        sum.files += 1;
        sum.added += a.parse::<u64>().unwrap_or(0);
        sum.removed += r.parse::<u64>().unwrap_or(0);
    }
    sum
}

/// The lines of a file's bytes: 0 for binary (a NUL byte) or empty; a last line without `\n` counts.
pub fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() || bytes.contains(&0) {
        return 0;
    }
    let n = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if bytes.ends_with(b"\n") { n } else { n + 1 }
}

fn run_git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// The summary of the repo at `dir`; `None` when `dir` is not a work tree or the `git` binary is missing.
pub fn summarize(dir: &Path) -> Option<DiffSummary> {
    run_git(dir, &["rev-parse", "--is-inside-work-tree"])?;
    // No commits yet: `HEAD` does not exist, so there are no tracked changes to count.
    let mut sum = run_git(dir, &["diff", "--numstat", "HEAD"]).map(|o| parse_numstat(&String::from_utf8_lossy(&o))).unwrap_or_default();
    if let Some(list) = run_git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        for name in list.split(|b| *b == 0).filter(|n| !n.is_empty()) {
            sum.files += 1;
            let path = dir.join(String::from_utf8_lossy(name).as_ref());
            if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_UNTRACKED_BYTES)
                && let Ok(bytes) = std::fs::read(&path)
            {
                sum.added += count_lines(&bytes);
            }
        }
    }
    Some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_sums_lines_and_files() {
        let s = parse_numstat("3\t1\tsrc/a.rs\n10\t0\tb.txt\n");
        assert_eq!(s, DiffSummary { added: 13, removed: 1, files: 2 });
    }

    #[test]
    fn numstat_counts_binary_files_without_lines() {
        let s = parse_numstat("-\t-\timg.png\n2\t5\tc.rs\n");
        assert_eq!(s, DiffSummary { added: 2, removed: 5, files: 2 });
    }

    #[test]
    fn numstat_ignores_blank_and_malformed_lines() {
        assert_eq!(parse_numstat(""), DiffSummary::default());
        assert_eq!(parse_numstat("\nnot numstat\n"), DiffSummary::default());
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
