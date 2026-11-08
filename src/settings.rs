//! The settings (K7a): the appearance choice, kept in a small JSON file. A missing or broken
//! file means the defaults. `CONTR_SETTINGS` points the file elsewhere (tests).

use std::path::PathBuf;

/// Which colours the whole app uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Appearance {
    /// Follow the system's light or dark appearance, live.
    #[default]
    System,
    Dark,
    Light,
}

impl Appearance {
    /// The order of the segmented control.
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Dark, Appearance::Light];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Dark => "Dark",
            Appearance::Light => "Light",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }

    fn from_id(id: &str) -> Option<Appearance> {
        Appearance::ALL.into_iter().find(|a| a.id() == id)
    }

    /// Whether the app is dark, given the system's appearance.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Appearance::System => system_dark,
            Appearance::Dark => true,
            Appearance::Light => false,
        }
    }
}

/// Where the settings file lives: `CONTR_SETTINGS`, else `~/Library/Application Support/contr-rune/settings.json`
/// on macOS, else `$XDG_CONFIG_HOME/contr-rune/settings.json` (default `~/.config`).
pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CONTR_SETTINGS").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
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
    Some(dir.join("contr-rune/settings.json"))
}

/// The file's text for `a`.
fn render(a: Appearance) -> String {
    format!("{{\n  \"appearance\": \"{}\"\n}}\n", a.id())
}

/// The appearance in the file's text: the value of the `"appearance"` key; `None` when it is
/// missing or not one of `system`, `dark`, `light`.
fn parse(text: &str) -> Option<Appearance> {
    let rest = &text[text.find("\"appearance\"")? + "\"appearance\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Appearance::from_id(&rest[..rest.find('"')?])
}

/// The saved appearance; System when the file is missing or broken.
pub fn load() -> Appearance {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse(&t)).unwrap_or_default()
}

/// Write the choice, creating the folder. Errors are returned for the caller to show.
pub fn save(a: Appearance) -> std::io::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, render(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_and_parse_round_trip() {
        for a in Appearance::ALL {
            assert_eq!(parse(&render(a)), Some(a));
        }
    }

    #[test]
    fn broken_or_unknown_text_has_no_choice() {
        for t in ["", "not json", "{}", "{\"appearance\": 3}", "{\"appearance\": \"sepia\"}", "{\"appearance\": \"dar"] {
            assert_eq!(parse(t), None, "{t:?}");
        }
        assert_eq!(parse("{ \"appearance\" :\"light\", \"x\": 1 }"), Some(Appearance::Light));
    }

    #[test]
    fn dark_follows_the_choice_or_the_system() {
        assert!(Appearance::System.is_dark(true));
        assert!(!Appearance::System.is_dark(false));
        assert!(Appearance::Dark.is_dark(false));
        assert!(!Appearance::Light.is_dark(true));
    }

    /// The env var steers `path`, `load` and `save`; one test owns it, so nothing races.
    #[test]
    fn save_then_load_through_the_env_path() {
        let dir = std::env::temp_dir().join(format!("contr-rune-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested/settings.json");
        // SAFETY: this is the only test that touches the process environment.
        unsafe { std::env::set_var("CONTR_SETTINGS", &file) };
        assert_eq!(path(), Some(file.clone()));
        assert_eq!(load(), Appearance::System, "a missing file falls back to System");
        save(Appearance::Light).unwrap();
        assert_eq!(load(), Appearance::Light);
        save(Appearance::Dark).unwrap();
        assert_eq!(load(), Appearance::Dark);
        std::fs::write(&file, "{ broken").unwrap();
        assert_eq!(load(), Appearance::System, "a broken file falls back to System");
        unsafe { std::env::remove_var("CONTR_SETTINGS") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
