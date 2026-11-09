//! Where contr keeps its files, Go-compatible (PARITY-PLAN K2). Every function has a testable
//! twin that takes the environment as a closure, so tests never touch the process environment.

use std::ffi::OsString;
use std::path::PathBuf;

/// Looks a variable up; `std::env::var_os` in the app, a table in tests.
type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn real_env(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

fn default_dir(env: Env) -> Option<PathBuf> {
    Some(PathBuf::from(env("HOME")?).join("Library/Application Support/contr"))
}

/// `CONTR_STATE_DIR`, else `~/Library/Application Support/contr`.
pub fn state_dir() -> Option<PathBuf> {
    state_dir_in(&real_env)
}

pub fn state_dir_in(env: Env) -> Option<PathBuf> {
    match env("CONTR_STATE_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => default_dir(env),
    }
}

/// `CONTR_AGENTS_CONFIG`, else `~/Library/Application Support/contr/agents.json`.
pub fn agents_config() -> Option<PathBuf> {
    agents_config_in(&real_env)
}

pub fn agents_config_in(env: Env) -> Option<PathBuf> {
    match env("CONTR_AGENTS_CONFIG") {
        Some(path) => Some(PathBuf::from(path)),
        None => Some(default_dir(env)?.join("agents.json")),
    }
}

/// `CONTR_OWNER_FILE`, else `~/Library/Application Support/contr/owner.json`.
pub fn owner_file() -> Option<PathBuf> {
    owner_file_in(&real_env)
}

pub fn owner_file_in(env: Env) -> Option<PathBuf> {
    match env("CONTR_OWNER_FILE") {
        Some(path) => Some(PathBuf::from(path)),
        None => Some(default_dir(env)?.join("owner.json")),
    }
}

/// Every character outside `[A-Za-z0-9_-]` becomes `_`, so an id is safe as a file name.
pub fn sanitize_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// `<state>/session/<id>.json`.
#[allow(dead_code)] // used by D7 (session file) and D10 (context file)
pub fn session_file(id: &str) -> Option<PathBuf> {
    session_file_in(&real_env, id)
}

#[allow(dead_code)] // used by D7 (session file) and D10 (context file)
pub fn session_file_in(env: Env, id: &str) -> Option<PathBuf> {
    Some(state_dir_in(env)?.join("session").join(format!("{}.json", sanitize_id(id))))
}

/// `<state>/usage/context/<id>.json`.
#[allow(dead_code)] // used by D7 (session file) and D10 (context file)
pub fn context_file(id: &str) -> Option<PathBuf> {
    context_file_in(&real_env, id)
}

#[allow(dead_code)] // used by D7 (session file) and D10 (context file)
pub fn context_file_in(env: Env, id: &str) -> Option<PathBuf> {
    Some(state_dir_in(env)?.join("usage/context").join(format!("{}.json", sanitize_id(id))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(*v))
    }

    #[test]
    fn variables_win_over_the_home_defaults() {
        let home = [("HOME", "/h")];
        assert_eq!(state_dir_in(&env(&home)), Some("/h/Library/Application Support/contr".into()));
        assert_eq!(agents_config_in(&env(&home)), Some("/h/Library/Application Support/contr/agents.json".into()));
        let set = [("HOME", "/h"), ("CONTR_STATE_DIR", "/s"), ("CONTR_AGENTS_CONFIG", "/a.json")];
        assert_eq!(state_dir_in(&env(&set)), Some("/s".into()));
        assert_eq!(agents_config_in(&env(&set)), Some("/a.json".into()));
        // The state variable does not move the profiles file (Go keeps them apart).
        let only_state = [("HOME", "/h"), ("CONTR_STATE_DIR", "/s")];
        assert_eq!(agents_config_in(&env(&only_state)), Some("/h/Library/Application Support/contr/agents.json".into()));
        assert_eq!(state_dir_in(&env(&[])), None);
        assert_eq!(owner_file_in(&env(&home)), Some("/h/Library/Application Support/contr/owner.json".into()));
        let owner = [("HOME", "/h"), ("CONTR_STATE_DIR", "/s"), ("CONTR_OWNER_FILE", "/o.json")];
        assert_eq!(owner_file_in(&env(&owner)), Some("/o.json".into()));
        assert_eq!(owner_file_in(&env(&only_state)), Some("/h/Library/Application Support/contr/owner.json".into()));
    }

    #[test]
    fn ids_are_sanitised_into_the_session_and_context_paths() {
        assert_eq!(sanitize_id("ab_C-9"), "ab_C-9");
        assert_eq!(sanitize_id("../a b:c"), "___a_b_c");
        let set = [("CONTR_STATE_DIR", "/s")];
        assert_eq!(session_file_in(&env(&set), "t/1"), Some("/s/session/t_1.json".into()));
        assert_eq!(context_file_in(&env(&set), "t/1"), Some("/s/usage/context/t_1.json".into()));
    }
}
