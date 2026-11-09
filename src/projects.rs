//! The saved projects (C5, K2): the open folders in order and the selected one, kept in a small
//! JSON file of our own (`{"version":1,"open":[path…],"selected":path}`; Go contr's file is not used).
//! A missing or broken file means no saved projects. `CONTR_PROJECTS_FILE` points the file
//! elsewhere (tests); a headless run without it never reads or writes the owner's real file.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq, Default)]
pub struct Saved {
    #[serde(default)]
    pub open: Vec<PathBuf>,
    /// The selected project; `None` for the first.
    #[serde(default)]
    pub selected: Option<PathBuf>,
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

fn render(open: &[&Path], selected: Option<&Path>) -> String {
    let v = serde_json::json!({ "version": 1, "open": open, "selected": selected });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

/// The saved projects; `None` when the file is missing, broken or lists no folder that still exists.
/// The folders that are gone are dropped.
pub fn load() -> Option<Saved> {
    let mut saved = parse(&std::fs::read_to_string(path()?).ok()?)?;
    saved.open.retain(|p| p.is_dir());
    (!saved.open.is_empty()).then_some(saved)
}

/// Write the open folders and the selected one (a temp file, then a rename), creating the folder.
pub fn save(open: &[&Path], selected: Option<&Path>) -> std::io::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, render(open, selected))?;
    std::fs::rename(tmp, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_and_parse_round_trip() {
        let (a, b) = (Path::new("/a"), Path::new("/b c"));
        let s = parse(&render(&[a, b], Some(b))).unwrap();
        assert_eq!(s, Saved { open: vec![a.into(), b.into()], selected: Some(b.into()) });
        assert_eq!(parse(&render(&[], None)).unwrap().selected, None);
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
        save(&[&a, &b], Some(&b)).unwrap();
        assert!(!file.with_extension("json.tmp").exists(), "the temp file is renamed away");
        let s = load().unwrap();
        assert_eq!((s.open, s.selected), (vec![a.clone(), b.clone()], Some(b.clone())));
        std::fs::remove_dir_all(&b).unwrap();
        assert_eq!(load().unwrap().open, vec![a.clone()], "a folder that is gone is dropped");
        std::fs::write(&file, "{ broken").unwrap();
        assert_eq!(load(), None, "a broken file has no projects");
        unsafe { std::env::remove_var("CONTR_PROJECTS_FILE") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
