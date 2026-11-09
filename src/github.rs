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

/// `gh <args>` in `dir`: Ok when it exits 0, else the first line of its stderr.
fn gh(dir: &Path, args: &[&str]) -> Result<(), String> {
    let program = program().ok_or("gh is not available")?;
    let out = Command::new(&program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
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
    gh(dir, &["pr", "view", "--web"])
}
