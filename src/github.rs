//! The `gh` commands the app runs (K5), in an agent's worktree. `CONTR_GH` names the program (tests
//! point it at a script); a headless run without it never runs a real `gh`.

use std::path::Path;
use std::process::{Command, Stdio};

/// The program to run: `CONTR_GH`, else `gh` (not in a headless run).
fn program() -> Option<String> {
    match std::env::var("CONTR_GH") {
        Ok(p) if !p.is_empty() => Some(p),
        _ if rune::is_headless() => None,
        _ => Some("gh".to_owned()),
    }
}

/// `gh <args>` in `dir`: its stdout when it exits 0, else the first line of its stderr.
fn gh(dir: &Path, args: &[&str]) -> Result<String, String> {
    let program = program().ok_or("gh is not available")?;
    let out = Command::new(&program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().next().map(str::trim).filter(|l| !l.is_empty()).unwrap_or("gh failed").to_owned())
}

/// Whether the branch in `dir` has a pull request.
pub fn has_pr(dir: &Path) -> bool {
    gh(dir, &["pr", "view", "--json", "number"]).is_ok()
}

/// Open the branch's pull request in the browser.
pub fn open_pr(dir: &Path) -> Result<(), String> {
    gh(dir, &["pr", "view", "--web"]).map(drop)
}

/// The commit at the head of pull request `pr` (`gh pr view <pr> --json headRefOid`).
pub fn pr_head(dir: &Path, pr: i64) -> Result<String, String> {
    let out = gh(dir, &["pr", "view", &pr.to_string(), "--json", "headRefOid"])?;
    let head: serde_json::Value = serde_json::from_str(&out).map_err(|e| format!("gh pr view: {e}"))?;
    match head["headRefOid"].as_str() {
        Some(sha) if !sha.is_empty() => Ok(sha.to_owned()),
        _ => Err(format!("pull request {pr} has no head commit")),
    }
}

/// `owner/repo` of a GitHub remote URL (`<user>@github.com:o/r.git`, `https://github.com/o/r`).
pub fn parse_remote(url: &str) -> Option<String> {
    let path = url.trim().split_once("github.com").map(|(_, p)| p)?;
    let path = path.strip_prefix([':', '/'])?.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && !s.contains('/');
    (ok(owner) && ok(repo)).then(|| path.to_owned())
}

/// `owner/repo` of the `origin` remote of the repo at `dir`, read from its config file (no
/// `git` binary; a worktree's `.git` file leads to the main repo's config). `None` without a
/// GitHub `origin`.
pub fn origin_repo(dir: &Path) -> Option<String> {
    let dot = dir.join(".git");
    let mut base = dot.clone();
    if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let target = Path::new(text.trim().strip_prefix("gitdir:")?.trim()).to_path_buf();
        base = if target.is_absolute() { target } else { dir.join(target) };
        if let Ok(common) = std::fs::read_to_string(base.join("commondir")) {
            base = base.join(common.trim());
        }
    }
    let config = std::fs::read_to_string(base.join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
        } else if in_origin && let Some(url) = line.strip_prefix("url").and_then(|r| r.trim_start().strip_prefix('=')) {
            return parse_remote(url);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_parse_to_owner_and_repo() {
        for (url, want) in [
            ("git@github.com:contursofran/contr.git", Some("contursofran/contr")),
            ("https://github.com/contursofran/contr", Some("contursofran/contr")),
            ("https://github.com/o/r.git/", Some("o/r")),
            ("https://gitlab.com/o/r.git", None),
            ("https://github.com/o", None),
        ] {
            assert_eq!(parse_remote(url).as_deref(), want, "{url}");
        }
    }
}
