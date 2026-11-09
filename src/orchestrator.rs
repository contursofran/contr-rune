//! A new orchestrator (A3): the `orchestrator` profile started in a folder, in this process
//! (PARITY-PLAN K3, go:cmd/contr/neworch.go). It calls `agents::spawn`, never the `contr` binary.

use std::path::{Path, PathBuf};

use crate::agents::{self, Spec};
use crate::agents_config::Config;
use crate::hc::Dialer;
use crate::hc::conn::Conn;
use crate::hc::watch::handshake;
use crate::sidebar::folder_name;

const PROFILE: &str = "orchestrator";
/// hc's limit for a terminal name.
const NAME_MAX: usize = 32;

/// `orch-<folder>` in the characters hc allows, with the `-MMSS` of the time as a suffix so a
/// second orchestrator in the same folder gets its own name; at most 32 characters.
pub fn name(dir: &Path, now_secs: u64) -> String {
    let mut base = String::new();
    for c in folder_name(dir).to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let suffix = format!("-{:02}{:02}", now_secs / 60 % 60, now_secs % 60);
    let mut name = format!("orch-{}", base.trim_matches('-'));
    name.truncate(NAME_MAX - suffix.len());
    format!("{}{suffix}", name.trim_end_matches('-'))
}

/// Starts an orchestrator in `dir` on a new connection to the core. `config` is the profiles
/// file; a file without an `orchestrator` profile gives an error that names it.
pub fn start(dialer: Option<&Dialer>, config: Option<PathBuf>, dir: &Path, now_secs: u64) -> Result<(), String> {
    let path = config.ok_or("no home folder to find agents.json in")?;
    let cfg = Config::load(&path)?;
    if !cfg.profiles.contains_key(PROFILE) {
        return Err(format!("no {PROFILE:?} profile in {}", path.display()));
    }
    let dialer = dialer.ok_or("hc is off")?;
    let link = dialer.dial().map_err(|e| e.to_string())?;
    let conn = Conn::new(link.reader, link.writer, None);
    handshake(&conn, false)?;
    let spec = Spec {
        name: name(dir, now_secs),
        profile: PROFILE.into(),
        role: PROFILE.into(),
        worktree: dir.display().to_string(),
        project: folder_name(dir),
        ..Spec::default()
    };
    let env_kit = std::env::var_os("CONTR_KIT");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    agents::spawn(&conn, &cfg, spec, env_kit.as_ref(), home.as_deref())?;
    drop(link.keep);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_go_rule() {
        let n = |p: &str| name(Path::new(p), 12 * 60 + 34);
        assert_eq!(n("/a/My Project"), "orch-my-project-1234");
        assert_eq!(n("/a/--x--"), "orch-x-1234");
        assert_eq!(n("/a/__Dev_1"), "orch-__dev_1-1234");
        let long = n("/a/abcdefghijklmnopqrstuvwxyz0123456789");
        assert_eq!(long, "orch-abcdefghijklmnopqrstuv-1234");
        assert_eq!(long.len(), 32);
        // A cut that ends in `-` drops it.
        assert_eq!(n("/a/abcdefghijklmnopqrstu-vwxyz"), "orch-abcdefghijklmnopqrstu-1234");
    }
}
