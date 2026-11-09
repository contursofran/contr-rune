//! The saved projects (C5, K2): the open folders in order and the selected one, kept in a small
//! JSON file of our own (`{"version":1,"open":[path…],"selected":path,"hidden":[path…],"recent":[{"path","at"}],"agent":address}`;
//! Go contr's file is not used). `hidden` (C6) and `recent` are new: a file without them still loads.
//! A missing or broken file means no saved projects. `CONTR_PROJECTS_FILE` points the file
//! elsewhere (tests); a headless run without it never reads or writes the owner's real file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A project that was hidden, and when (RFC 3339, UTC).
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Recent {
    pub path: PathBuf,
    pub at: String,
}

/// The most entries `recent` keeps.
pub const MAX_RECENT: usize = 10;

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct Saved {
    #[serde(default)]
    pub open: Vec<PathBuf>,
    /// The selected project; `None` for the first.
    #[serde(default)]
    pub selected: Option<PathBuf>,
    /// Projects taken off the sidebar in the projects modal (C6); they keep their place in the list.
    #[serde(default)]
    pub hidden: Vec<PathBuf>,
    /// Hidden projects, newest first, at most `MAX_RECENT`.
    #[serde(default)]
    pub recent: Vec<Recent>,
    /// The selected agent's address (Go D173); `None` for no agent.
    #[serde(default)]
    pub agent: Option<String>,
}

/// Where the file lives: `CONTR_PROJECTS_FILE`, else `~/Library/Application Support/contr-rune/projects.json`
/// on macOS, else `$XDG_CONFIG_HOME/contr-rune/projects.json` (default `~/.config`). A headless run
/// has no file unless `CONTR_PROJECTS_FILE` is set.
pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CONTR_PROJECTS_FILE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if rune::is_headless() {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
            Some(x) => PathBuf::from(x),
            None => home?.join(".config"),
        }
    };
    Some(dir.join("contr-rune/projects.json"))
}

fn parse(text: &str) -> Option<Saved> {
    serde_json::from_str(text).ok()
}

fn render(saved: &Saved) -> String {
    let mut v = serde_json::to_value(saved).unwrap_or_default();
    v["version"] = 1.into();
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// The saved projects; `None` when the file is missing, broken or lists no open folder that still exists.
/// The folders that are gone are dropped.
pub fn load() -> Option<Saved> {
    let mut saved = parse(&std::fs::read_to_string(path()?).ok()?)?;
    saved.open.retain(|p| p.is_dir());
    saved.hidden.retain(|p| p.is_dir());
    saved.recent.retain(|r| r.path.is_dir());
    (!saved.open.is_empty()).then_some(saved)
}

/// Write the projects (a temp file, then a rename), creating the folder.
pub fn save(saved: &Saved) -> std::io::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, render(saved))?;
    std::fs::rename(tmp, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn render_and_parse_round_trip() {
        let (a, b) = (Path::new("/a"), Path::new("/b c"));
        let saved = Saved {
            open: vec![a.into(), b.into()],
            selected: Some(b.into()),
            hidden: vec!["/h".into()],
            recent: vec![Recent { path: "/h".into(), at: "2026-11-09T10:00:00Z".into() }],
            agent: Some("t17".into()),
        };
        assert_eq!(parse(&render(&saved)).unwrap(), saved);
        assert_eq!(parse(&render(&Saved::default())).unwrap().selected, None);
    }

    #[test]
    fn a_file_from_before_hidden_and_recent_still_loads() {
        let s = parse(r#"{"version":1,"open":["/a"],"selected":"/a"}"#).unwrap();
        assert_eq!((s.open, s.hidden, s.recent, s.agent), (vec![PathBuf::from("/a")], vec![], vec![], None));
    }

    #[test]
    fn broken_text_has_no_projects() {
        for t in ["", "not json","{\"open\": 3}", "{\"open\": [\"/a\""] {
            assert_eq!(parse(t), None, "{t:?}");
        }
        assert_eq!(parse("{}").unwrap().open, Vec::<PathBuf>::new());
    }

    /// The env var steers `path`, `load` and `save`; one test owns it, so nothing races.
    #[test]
    fn save_then_load_through_the_env_path() {
        let dir = std::env::temp_dir().join(format!("contr-rune-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let file = dir.join("nested/projects.json");
        // SAFETY: this is the only test that touches the process environment.
        unsafe { std::env::set_var("CONTR_PROJECTS_FILE", &file) };
        assert_eq!(load(), None, "a missing file has no projects");
        let recent = vec![Recent { path: b.clone(), at: "2026-11-09T10:00:00Z".into() }];
        save(&Saved { open: vec![a.clone(), b.clone()], selected: Some(b.clone()), hidden: vec![b.clone()], recent: recent.clone(), agent: Some("t3".into()) }).unwrap();
        assert!(!file.with_extension("json.tmp").exists(), "the temp file is renamed away");
        let s = load().unwrap();
        assert_eq!((s.open, s.selected, s.hidden, s.recent, s.agent), (vec![a.clone(), b.clone()], Some(b.clone()), vec![b.clone()], recent, Some("t3".to_owned())));
        std::fs::remove_dir_all(&b).unwrap();
        let s = load().unwrap();
        assert_eq!((s.open, s.hidden, s.recent), (vec![a.clone()], vec![], vec![]), "a folder that is gone is dropped");
        std::fs::write(&file, "{ broken").unwrap();
        assert_eq!(load(), None, "a broken file has no projects");
        unsafe { std::env::remove_var("CONTR_PROJECTS_FILE") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
