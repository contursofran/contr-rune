//! The session file of one hc terminal: the agent's todos and artifacts (PARITY-PLAN K2;
//! Go: internal/session). The kit's hooks write it (`contr-rune hook post-tool`); the app reads it.
//! The shape is Go-compatible.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

/// The file format version.
pub const SCHEMA: u32 = 1;
/// The most artifacts a file keeps.
pub const MAX_ARTIFACTS: usize = 20;
/// Starts the text of a todo that came from the owner's prompt.
pub const YOU_PREFIX: &str = "[you] ";

pub const PENDING: &str = "pending";

/// One item of the agent's todo list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Todo {
    /// The task id of the task tools; absent for `TodoWrite` items.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub text: String,
    pub active: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
    pub you: bool,
}

impl Todo {
    /// A todo from the texts, with the `[you] ` prefix cut into `you`.
    pub fn new(text: &str, active: &str, status: &str) -> Todo {
        let mut todo = Todo { text: text.into(), active: active.into(), status: status.into(), ..Todo::default() };
        if let Some(rest) = text.strip_prefix(YOU_PREFIX) {
            todo.text = rest.into();
            todo.you = true;
        }
        if let Some(rest) = active.strip_prefix(YOU_PREFIX) {
            todo.active = rest.into();
            todo.you = true;
        }
        todo
    }
}

/// One artifact the agent published, edited or read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Artifact {
    pub slug: String,
    pub title: String,
    pub url: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub schema: u32,
    pub updated_at: u64,
    #[serde(deserialize_with = "null_is_empty")]
    pub todos: Vec<Todo>,
    #[serde(deserialize_with = "null_is_empty")]
    pub artifacts: Vec<Artifact>,
}

/// Go writes a nil slice as `null`.
fn null_is_empty<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

impl Session {
    /// Puts `new` at the top, drops an older entry with the same slug and keeps at most
    /// `MAX_ARTIFACTS`. An empty title or URL keeps the old value.
    pub fn add_artifact(&mut self, new: Artifact) {
        let mut out = vec![new];
        for old in std::mem::take(&mut self.artifacts) {
            if old.slug == out[0].slug {
                if out[0].title.is_empty() {
                    out[0].title = old.title;
                }
                if out[0].url.is_empty() {
                    out[0].url = old.url;
                }
            } else {
                out.push(old);
            }
        }
        out.truncate(MAX_ARTIFACTS);
        self.artifacts = out;
    }
}

/// The session at `path`. A missing file gives an empty session; a file that is not JSON or has
/// another schema is an error.
pub fn read(path: &Path) -> Result<Session, String> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Session { schema: SCHEMA, ..Session::default() }),
        Err(e) => return Err(e.to_string()),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let session: Session = serde_json::from_slice(&data).map_err(|e| format!("session file {name}: {e}"))?;
    if session.schema != SCHEMA {
        return Err(format!("session file {name}: schema {}, want {SCHEMA}", session.schema));
    }
    Ok(session)
}

/// Saves `session` at `path` through a temp file and a rename, so a reader never sees half a file.
pub fn write(path: &Path, session: &mut Session) -> io::Result<()> {
    session.schema = SCHEMA;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_vec_pretty(session).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(slug: &str, title: &str, url: &str, at: u64) -> Artifact {
        Artifact { slug: slug.into(), title: title.into(), url: url.into(), updated_at: at }
    }

    fn todo(text: &str, active: &str, you: bool) -> Todo {
        Todo { text: text.into(), active: active.into(), status: "pending".into(), you, ..Todo::default() }
    }

    #[test]
    fn you_prefix_moves_into_the_flag() {
        for (text, active, want) in [
            ("[you] fix it", "", todo("fix it", "", true)),
            ("fix it", "[you] fixing it", todo("fix it", "fixing it", true)),
            ("fix it", "fixing", todo("fix it", "fixing", false)),
            ("[you]fix", "", todo("[you]fix", "", false)),
        ] {
            assert_eq!(Todo::new(text, active, "pending"), want);
        }
    }

    #[test]
    fn artifact_goes_to_the_top_keeps_old_title_and_url_and_caps_at_20() {
        let mut s = Session { artifacts: vec![art("a", "A", "u-a", 1), art("b", "B", "u-b", 1)], ..Session::default() };
        s.add_artifact(art("b", "", "", 2));
        assert_eq!(s.artifacts, vec![art("b", "B", "u-b", 2), art("a", "A", "u-a", 1)]);
        s.add_artifact(art("b", "B2", "", 3));
        assert_eq!(s.artifacts[0], art("b", "B2", "u-b", 3));
        for i in 0..25 {
            s.add_artifact(art(&format!("n{i}"), "", "", 4));
        }
        assert_eq!(s.artifacts.len(), MAX_ARTIFACTS);
        assert_eq!(s.artifacts[0].slug, "n24");
        assert_eq!(s.artifacts[19].slug, "n5");
    }

    #[test]
    fn file_round_trips_in_the_go_shape() {
        let dir = std::env::temp_dir().join(format!("contr-rune-session-file-{}", std::process::id()));
        let path = dir.join("session").join("t1.json");
        assert_eq!(read(&path).unwrap(), Session { schema: 1, ..Session::default() });
        let mut s = Session { updated_at: 5, todos: vec![Todo::new("[you] a", "", "pending"), Todo { id: "7".into(), ..Todo::new("b", "", "completed") }], ..Session::default() };
        write(&path, &mut s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["schema"], 1);
        assert!(v["todos"][0].get("id").is_none());
        assert_eq!(v["todos"][1]["id"], "7");
        assert_eq!(v["artifacts"], serde_json::json!([]));
        assert_eq!(read(&path).unwrap(), s);
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temp file left");
        // Go's nil slices are null; another schema and bad JSON are errors.
        std::fs::write(&path, r#"{"schema":1,"updated_at":2,"todos":null,"artifacts":null}"#).unwrap();
        assert_eq!(read(&path).unwrap().updated_at, 2);
        std::fs::write(&path, r#"{"schema":2}"#).unwrap();
        assert!(read(&path).unwrap_err().contains("schema 2"));
        std::fs::write(&path, "nope").unwrap();
        assert!(read(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
