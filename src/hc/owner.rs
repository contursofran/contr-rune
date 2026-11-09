//! The owner's questions and to-dos, in the `owner.json` that Go contr shares (Contract C7).
//!
//! Go contr's MCP tools write the file; this app reads it and writes the owner's answers. Writes
//! take an exclusive lock on `<file>.lock`, read the whole file, change one item and replace the
//! file through `<file>.tmp` and a rename. The file is kept as a JSON value, so fields this app does
//! not know survive a write.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::paths;

/// A decision that waits for the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Question {
    pub id: String,
    pub project: String,
    /// The hc terminal that asked; the answer is sent there.
    pub coordinator: String,
    pub from: String,
    pub title: String,
    pub question: String,
    pub options: Vec<String>,
    /// Index into `options`.
    pub recommended: Option<usize>,
    /// What each option means; `impacts[i]` is for `options[i]`.
    pub impacts: Vec<String>,
    /// Issue or PR numbers, file paths and URLs that help to decide.
    pub refs: Vec<String>,
    /// A short excerpt of the text that raised the question.
    pub source: String,
    pub answer: String,
}

impl Question {
    pub fn is_open(&self) -> bool {
        self.answer.is_empty()
    }
}

/// Something the owner must do (`do`) or should know (`know`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Todo {
    pub id: String,
    pub project: String,
    pub coordinator: String,
    pub from: String,
    pub kind: String,
    pub text: String,
    /// When it was pinned (RFC 3339, as Go wrote it).
    pub pinned: String,
    /// When it was done (RFC 3339); Go writes the zero time `0001-01-01…` for an open one.
    pub done: Option<String>,
}

impl Todo {
    pub fn is_open(&self) -> bool {
        is_unset_time(self.done.as_deref())
    }
}

/// A time field Go left out, null or wrote as its zero time.
fn is_unset_time(t: Option<&str>) -> bool {
    t.is_none_or(|t| t.is_empty() || t.starts_with("0001-01-01"))
}

/// What the file holds now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct OwnerState {
    pub questions: Vec<Question>,
    pub todos: Vec<Todo>,
}

impl OwnerState {
    /// The open questions of `project`, in file order.
    pub fn open_questions(&self, project: &str) -> Vec<&Question> {
        self.questions
            .iter()
            .filter(|q| q.is_open() && q.project == project)
            .collect()
    }

    /// The open to-dos of `project`, in file order.
    pub fn open_todos(&self, project: &str) -> Vec<&Todo> {
        self.todos
            .iter()
            .filter(|t| t.is_open() && t.project == project)
            .collect()
    }
}

/// Where the owner's items live. `OwnerFile` is the Go contr file; another source can sit behind
/// the same trait later.
pub trait OwnerSource: Send + Sync {
    fn read(&self) -> io::Result<OwnerState>;

    /// Record the owner's answer to an open question. Returns the coordinator terminal to tell,
    /// when the question has one.
    ///
    /// # Errors
    /// `InvalidInput` when the text is empty or `id` is not an open question (nothing changes).
    fn answer(&self, id: &str, text: &str) -> io::Result<Option<String>>;

    /// Mark an open to-do done. Returns its coordinator terminal, when it has one.
    ///
    /// # Errors
    /// `InvalidInput` when `id` is not an open to-do (nothing changes).
    fn done(&self, id: &str) -> io::Result<Option<String>>;

    /// A value that changes when the content changes (the file's modification time).
    fn stamp(&self) -> Option<SystemTime>;
}

/// `owner.json` of Go contr.
#[derive(Clone, Debug)]
pub struct OwnerFile {
    path: PathBuf,
}

impl OwnerFile {
    pub fn new(path: PathBuf) -> OwnerFile {
        OwnerFile { path }
    }

    /// `CONTR_OWNER_FILE`, else `~/Library/Application Support/contr/owner.json`.
    pub fn from_env() -> Option<OwnerFile> {
        paths::owner_file().map(OwnerFile::new)
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(suffix);
        PathBuf::from(name)
    }

    /// The whole file as JSON; a missing file is empty.
    fn load(&self) -> io::Result<Value> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", self.path.display()),
                )
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Ok(json!({"questions": [], "todos": [], "next_question": 1, "next_todo": 1}))
            }
            Err(e) => Err(e),
        }
    }

    /// Run `change` on the file's content under the lock and write the result.
    fn update<R>(&self, change: impl FnOnce(&mut Value) -> io::Result<R>) -> io::Result<R> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.sibling(".lock"))?;
        lock.lock()?;
        let mut value = self.load()?;
        let result = change(&mut value)?;
        let tmp = self.sibling(".tmp");
        let mut out = create_private(&tmp)?;
        let mut data = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
        data.push(b'\n');
        out.write_all(&data)?;
        out.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        // Closing `lock` releases it.
        drop(lock);
        Ok(result)
    }
}

/// A new file only the owner can read (the file holds the owner's decisions).
fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// The item of `list` whose id is `id`, if `open` accepts it.
fn open_item<'a>(
    value: &'a mut Value,
    list: &str,
    id: &str,
    open: impl Fn(&Value) -> bool,
) -> Option<&'a mut Value> {
    value
        .get_mut(list)?
        .as_array_mut()?
        .iter_mut()
        .find(|item| item["id"] == id && open(item))
}

fn coordinator_of(item: &Value) -> Option<String> {
    item["coordinator"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

/// A question to add; `ask` gives it its id.
#[derive(Clone, Debug, Default)]
pub struct NewQuestion {
    pub project: String,
    pub coordinator: String,
    pub from: String,
    pub title: String,
    pub question: String,
    pub options: Vec<String>,
    pub recommended: Option<usize>,
    pub impacts: Vec<String>,
    pub refs: Vec<String>,
    pub source: String,
}

/// A to-do to add; `pin` gives it its id.
#[derive(Clone, Debug, Default)]
pub struct NewTodo {
    pub project: String,
    pub coordinator: String,
    pub from: String,
    /// `do` (the default when empty) or `know`.
    pub kind: String,
    pub text: String,
}

/// Sets `key` to `value` unless it is empty, as Go's `omitempty`.
fn put(item: &mut Value, key: &str, value: Value) {
    let empty = match &value {
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    };
    if !empty {
        item[key] = value;
    }
}

/// Takes the next number of `counter` (`next_question` or `next_todo`; Go starts at 1).
fn next_id(value: &mut Value, counter: &str, prefix: &str) -> String {
    let n = value[counter].as_u64().filter(|n| *n >= 1).unwrap_or(1);
    value[counter] = json!(n + 1);
    format!("{prefix}{n}")
}

/// Appends `item` to the array `list` (made when it is missing or null).
fn push(value: &mut Value, list: &str, item: Value) {
    if !value[list].is_array() {
        value[list] = json!([]);
    }
    if let Some(items) = value[list].as_array_mut() {
        items.push(item);
    }
}

/// Every id of `ids` must be an open item of `list`, else the error names the first that is not.
fn all_open(value: &Value, list: &str, ids: &[String], what: &str, open: impl Fn(&Value) -> bool) -> io::Result<()> {
    let items = value[list].as_array().map_or(&[][..], Vec::as_slice);
    match ids.iter().find(|id| !items.iter().any(|i| i["id"] == id.as_str() && open(i))) {
        Some(id) => Err(invalid(format!("no open {what} {id}"))),
        None => Ok(()),
    }
}

fn question_is_open(q: &Value) -> bool {
    q["answer"].as_str().is_none_or(str::is_empty)
}

fn todo_is_open(t: &Value) -> bool {
    is_unset_time(t["done"].as_str())
}

impl OwnerFile {
    /// The open questions and to-dos as the file holds them (Go's field names), those of
    /// `project` only, or of all projects when it is `None`.
    pub fn open_items(&self, project: Option<&str>) -> io::Result<(Vec<Value>, Vec<Value>)> {
        let v = self.load()?;
        let pick = |list: &str, open: fn(&Value) -> bool| -> Vec<Value> {
            let items = v[list].as_array().map_or(&[][..], Vec::as_slice);
            items.iter().filter(|i| open(i) && project.is_none_or(|p| i["project"].as_str().unwrap_or_default() == p)).cloned().collect()
        };
        Ok((pick("questions", question_is_open), pick("todos", todo_is_open)))
    }

    /// Adds a question and returns its id (`D<n>`).
    ///
    /// # Errors
    /// `InvalidInput` for an empty question or a `recommended` that is no option index.
    pub fn ask(&self, q: NewQuestion) -> io::Result<String> {
        if q.question.is_empty() {
            return Err(invalid("a question needs text".to_owned()));
        }
        if let Some(r) = q.recommended.filter(|r| *r >= q.options.len()) {
            return Err(invalid(format!("recommended {r} is not an option index")));
        }
        self.update(|v| {
            let id = next_id(v, "next_question", "D");
            let mut item = json!({"id": id, "project": q.project, "question": q.question, "asked": rfc3339_now()});
            put(&mut item, "coordinator", json!(q.coordinator));
            put(&mut item, "from", json!(q.from));
            put(&mut item, "title", json!(q.title));
            put(&mut item, "options", json!(q.options));
            if let Some(r) = q.recommended {
                item["recommended"] = json!(r);
            }
            put(&mut item, "impacts", json!(q.impacts));
            put(&mut item, "refs", json!(q.refs));
            put(&mut item, "source", json!(q.source));
            push(v, "questions", item);
            Ok(id)
        })
    }

    /// Adds a to-do and returns its id (`T<n>`).
    ///
    /// # Errors
    /// `InvalidInput` for empty text or a kind that is not `do` or `know`.
    pub fn pin(&self, mut t: NewTodo) -> io::Result<String> {
        if t.text.is_empty() {
            return Err(invalid("a to-do needs text".to_owned()));
        }
        if t.kind.is_empty() {
            t.kind = "do".to_owned();
        }
        if t.kind != "do" && t.kind != "know" {
            return Err(invalid(format!("kind {:?} is not do or know", t.kind)));
        }
        self.update(|v| {
            let id = next_id(v, "next_todo", "T");
            let mut item = json!({"id": id, "project": t.project, "kind": t.kind, "text": t.text, "pinned": rfc3339_now()});
            put(&mut item, "coordinator", json!(t.coordinator));
            put(&mut item, "from", json!(t.from));
            push(v, "todos", item);
            Ok(id)
        })
    }

    /// Records `answer` for open questions. `by` is `owner` or `coordinator`. When an id is not an
    /// open question nothing changes.
    ///
    /// # Errors
    /// `InvalidInput` for no ids, an empty answer, another `by` or an id that is not open.
    pub fn decide(&self, ids: &[String], answer: &str, by: &str) -> io::Result<()> {
        if ids.is_empty() {
            return Err(invalid("no question ids".to_owned()));
        }
        if answer.is_empty() {
            return Err(invalid("an answer needs text".to_owned()));
        }
        if by != "owner" && by != "coordinator" {
            return Err(invalid(format!("by {by:?} is not owner or coordinator")));
        }
        self.update(|v| {
            all_open(v, "questions", ids, "question", question_is_open)?;
            let now = rfc3339_now();
            for q in v["questions"].as_array_mut().into_iter().flatten() {
                if ids.iter().any(|id| q["id"] == id.as_str()) {
                    q["answer"] = json!(answer);
                    q["answered_by"] = json!(by);
                    q["answered"] = json!(now);
                }
            }
            Ok(())
        })
    }

    /// Marks open to-dos done. When an id is not an open to-do nothing changes.
    ///
    /// # Errors
    /// `InvalidInput` for no ids or an id that is not open.
    pub fn unpin(&self, ids: &[String]) -> io::Result<()> {
        if ids.is_empty() {
            return Err(invalid("no to-do ids".to_owned()));
        }
        self.update(|v| {
            all_open(v, "todos", ids, "to-do", todo_is_open)?;
            let now = rfc3339_now();
            for t in v["todos"].as_array_mut().into_iter().flatten() {
                if ids.iter().any(|id| t["id"] == id.as_str()) {
                    t["done"] = json!(now);
                }
            }
            Ok(())
        })
    }
}

impl OwnerSource for OwnerFile {
    fn read(&self) -> io::Result<OwnerState> {
        serde_json::from_value(self.load()?).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", self.path.display()),
            )
        })
    }

    fn answer(&self, id: &str, text: &str) -> io::Result<Option<String>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid("an answer needs text".to_owned()));
        }
        self.update(|v| {
            let item = open_item(v, "questions", id, |q| {
                q["answer"].as_str().is_none_or(str::is_empty)
            })
            .ok_or_else(|| invalid(format!("no open question {id}")))?;
            item["answer"] = json!(text);
            item["answered_by"] = json!("owner");
            item["answered"] = json!(rfc3339_now());
            Ok(coordinator_of(item))
        })
    }

    fn done(&self, id: &str) -> io::Result<Option<String>> {
        self.update(|v| {
            let item = open_item(v, "todos", id, |t| is_unset_time(t["done"].as_str()))
                .ok_or_else(|| invalid(format!("no open to-do {id}")))?;
            item["done"] = json!(rfc3339_now());
            Ok(coordinator_of(item))
        })
    }

    fn stamp(&self) -> Option<SystemTime> {
        fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }
}

/// The current time as RFC 3339 in UTC, to the second (what Go's `time.Time` parses).
fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

fn rfc3339(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FILE: &str = r#"{
      "questions": [
        {"id": "D1", "project": "contr", "coordinator": "t3", "question": "Ship it?", "options": ["yes", "no"], "recommended": 1, "asked": "2026-11-01T10:00:00Z", "future_field": {"keep": true}},
        {"id": "D2", "project": "contr", "question": "Which colour?", "asked": "2026-11-01T11:00:00Z"},
        {"id": "D3", "project": "other", "question": "Old", "answer": "x", "answered_by": "owner", "answered": "2026-11-02T10:00:00Z"}
      ],
      "todos": [
        {"id": "T1", "project": "contr", "coordinator": "t3", "kind": "do", "text": "Rotate the key", "pinned": "2026-11-01T10:00:00Z", "done": "0001-01-01T00:00:00Z"},
        {"id": "T2", "project": "contr", "kind": "know", "text": "FYI", "pinned": "2026-11-01T10:00:00Z", "done": "2026-11-02T10:00:00Z"}
      ],
      "next_question": 4, "next_todo": 3, "extra": [1, 2]
    }"#;

    fn temp_file(content: Option<&str>) -> OwnerFile {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "contr-rune-owner-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("owner.json");
        if let Some(c) = content {
            fs::write(&path, c).unwrap();
        }
        OwnerFile::new(path)
    }

    fn raw(f: &OwnerFile) -> Value {
        serde_json::from_slice(&fs::read(f.path()).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_file_is_empty() {
        let f = temp_file(None);
        assert_eq!(f.read().unwrap(), OwnerState::default());
        assert_eq!(f.stamp(), None);
    }

    #[test]
    fn open_items_are_per_project_and_read_go_zero_times() {
        let f = temp_file(Some(FILE));
        let st = f.read().unwrap();
        assert_eq!(
            st.open_questions("contr")
                .iter()
                .map(|q| q.id.as_str())
                .collect::<Vec<_>>(),
            ["D1", "D2"]
        );
        assert!(st.open_questions("other").is_empty(), "D3 is answered");
        assert_eq!(
            st.open_todos("contr")
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["T1"],
            "the zero time is open, a real time is done"
        );
        assert_eq!(st.questions[0].recommended, Some(1));
    }

    #[test]
    fn an_answer_round_trips_and_keeps_unknown_fields() {
        let f = temp_file(Some(FILE));
        assert_eq!(f.answer("D1", "  no  ").unwrap(), Some("t3".to_owned()));
        let v = raw(&f);
        let d1 = &v["questions"][0];
        assert_eq!(
            (d1["answer"].as_str(), d1["answered_by"].as_str()),
            (Some("no"), Some("owner"))
        );
        assert!(
            d1["answered"]
                .as_str()
                .is_some_and(|t| t.ends_with('Z') && t.contains('T')),
            "{d1}"
        );
        assert_eq!(d1["future_field"], json!({"keep": true}));
        assert_eq!(v["extra"], json!([1, 2]));
        assert_eq!(v["next_question"], 4);
        assert_eq!(
            v["questions"][1]["answer"],
            Value::Null,
            "other questions are untouched"
        );
        assert_eq!(f.read().unwrap().open_questions("contr").len(), 1);
        assert!(
            !f.sibling(".tmp").exists(),
            "the temp file was renamed away"
        );
        assert!(f.sibling(".lock").exists());
    }

    #[test]
    fn a_question_without_a_coordinator_has_none_to_tell() {
        let f = temp_file(Some(FILE));
        assert_eq!(f.answer("D2", "blue").unwrap(), None);
    }

    #[test]
    fn done_marks_a_todo_and_returns_its_coordinator() {
        let f = temp_file(Some(FILE));
        assert_eq!(f.done("T1").unwrap(), Some("t3".to_owned()));
        assert!(f.read().unwrap().open_todos("contr").is_empty());
        assert_ne!(raw(&f)["todos"][0]["done"], "0001-01-01T00:00:00Z");
    }

    #[test]
    fn deciding_errors_change_nothing() {
        let f = temp_file(Some(FILE));
        let before = fs::read(f.path()).unwrap();
        let kind = |r: io::Result<Option<String>>| r.unwrap_err().kind();
        assert_eq!(
            kind(f.answer("D9", "x")),
            io::ErrorKind::InvalidInput,
            "no such question"
        );
        assert_eq!(
            kind(f.answer("D3", "x")),
            io::ErrorKind::InvalidInput,
            "already answered"
        );
        assert_eq!(
            kind(f.answer("D1", "   ")),
            io::ErrorKind::InvalidInput,
            "empty text"
        );
        assert_eq!(
            kind(f.done("T2")),
            io::ErrorKind::InvalidInput,
            "already done"
        );
        assert_eq!(
            kind(f.done("D1")),
            io::ErrorKind::InvalidInput,
            "not a to-do"
        );
        assert_eq!(fs::read(f.path()).unwrap(), before);
        let msg = f.answer("D9", "x").unwrap_err().to_string();
        assert!(msg.contains("D9"), "{msg}");
    }

    #[test]
    fn a_broken_file_is_an_error_not_an_empty_state() {
        let f = temp_file(Some("{not json"));
        assert_eq!(f.read().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            f.answer("D1", "x").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            fs::read_to_string(f.path()).unwrap(),
            "{not json",
            "a broken file is left alone"
        );
    }

    #[test]
    fn writers_in_parallel_all_land_because_of_the_lock() {
        let questions: Vec<Value> = (1..=12)
            .map(|i| json!({"id": format!("D{i}"), "project": "p", "question": "?"}))
            .collect();
        let f = Arc::new(temp_file(Some(
            &json!({"questions": questions, "todos": []}).to_string(),
        )));
        let threads: Vec<_> = (1..=12)
            .map(|i| {
                let f = f.clone();
                std::thread::spawn(move || f.answer(&format!("D{i}"), "ok").unwrap())
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(
            f.read().unwrap().open_questions("p").is_empty(),
            "every answer survived"
        );
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ask_appends_with_the_next_id_and_keeps_the_rest() {
        let f = temp_file(Some(FILE));
        let q = NewQuestion {
            project: "contr".into(),
            coordinator: "t9".into(),
            question: "Which?".into(),
            options: ids(&["a", "b"]),
            recommended: Some(0),
            impacts: ids(&["fast"]),
            refs: ids(&["#3"]),
            source: "log".into(),
            ..NewQuestion::default()
        };
        assert_eq!(f.ask(q).unwrap(), "D4");
        assert_eq!(f.ask(NewQuestion { project: "contr".into(), question: "Plain?".into(), ..NewQuestion::default() }).unwrap(), "D5");
        let v = raw(&f);
        assert_eq!((v["next_question"].clone(), v["extra"].clone()), (json!(6), json!([1, 2])));
        assert_eq!(v["questions"][0]["future_field"], json!({"keep": true}));
        let d4 = &v["questions"][3];
        for (key, want) in [("id", json!("D4")), ("coordinator", json!("t9")), ("recommended", json!(0)), ("impacts", json!(["fast"])), ("refs", json!(["#3"])), ("source", json!("log"))] {
            assert_eq!(d4[key], want, "{key}");
        }
        assert!(d4["asked"].as_str().is_some_and(|t| t.ends_with('Z')));
        let d5 = v["questions"][4].as_object().unwrap();
        let mut keys: Vec<_> = d5.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["asked", "id", "project", "question"], "empty fields are left out, as Go omitempty");
        let st = f.read().unwrap();
        assert_eq!(st.questions[3].impacts, ["fast"]);
        assert_eq!((st.questions[3].refs.clone(), st.questions[3].source.as_str()), (ids(&["#3"]), "log"));
        assert_eq!(st.open_questions("contr").len(), 4);
    }

    #[test]
    fn a_new_file_starts_at_one_and_pin_counts_todos_apart() {
        let f = temp_file(None);
        let ask = |q: &str| f.ask(NewQuestion { question: q.into(), ..NewQuestion::default() }).unwrap();
        assert_eq!((ask("a").as_str(), ask("b").as_str()), ("D1", "D2"));
        let todo = |kind: &str| f.pin(NewTodo { project: "p".into(), kind: kind.into(), text: "x".into(), ..NewTodo::default() }).unwrap();
        assert_eq!((todo("").as_str(), todo("know").as_str()), ("T1", "T2"));
        let st = f.read().unwrap();
        assert_eq!((st.todos[0].kind.as_str(), st.todos[1].kind.as_str()), ("do", "know"));
        assert_eq!(st.open_todos("p").len(), 2);
        assert_eq!(raw(&f)["todos"][0].get("done"), None, "an open to-do has no done time");
    }

    #[test]
    fn adding_errors_change_nothing() {
        let f = temp_file(Some(FILE));
        let before = fs::read(f.path()).unwrap();
        let q = |question: &str, options: &[&str], recommended| NewQuestion { question: question.into(), options: ids(options), recommended, ..NewQuestion::default() };
        let t = |kind: &str, text: &str| NewTodo { kind: kind.into(), text: text.into(), ..NewTodo::default() };
        let errors = [
            f.ask(q("", &[], None)).unwrap_err(),
            f.ask(q("?", &["a"], Some(1))).unwrap_err(),
            f.ask(q("?", &[], Some(0))).unwrap_err(),
            f.pin(t("do", "")).unwrap_err(),
            f.pin(t("later", "x")).unwrap_err(),
        ];
        assert!(errors.iter().all(|e| e.kind() == io::ErrorKind::InvalidInput), "{errors:?}");
        assert_eq!(fs::read(f.path()).unwrap(), before);
    }

    #[test]
    fn decide_and_unpin_take_every_id_or_none() {
        let f = temp_file(Some(FILE));
        let before = fs::read(f.path()).unwrap();
        let by = |r: io::Result<()>| r.unwrap_err().to_string();
        assert!(by(f.decide(&ids(&["D1", "D3"]), "x", "owner")).contains("no open question D3"));
        assert!(by(f.decide(&[], "x", "owner")).contains("no question ids"));
        assert!(by(f.decide(&ids(&["D1"]), "", "owner")).contains("needs text"));
        assert!(by(f.decide(&ids(&["D1"]), "x", "me")).contains("not owner or coordinator"));
        assert!(by(f.unpin(&ids(&["T1", "T2"]))).contains("no open to-do T2"));
        assert!(by(f.unpin(&[])).contains("no to-do ids"));
        assert_eq!(fs::read(f.path()).unwrap(), before, "a failed call writes nothing");

        f.decide(&ids(&["D1", "D2"]), "go", "coordinator").unwrap();
        let v = raw(&f);
        for i in 0..2 {
            assert_eq!((v["questions"][i]["answer"].as_str(), v["questions"][i]["answered_by"].as_str()), (Some("go"), Some("coordinator")));
        }
        assert_eq!(v["questions"][0]["future_field"], json!({"keep": true}));
        assert!(f.read().unwrap().open_questions("contr").is_empty());
        f.unpin(&ids(&["T1"])).unwrap();
        assert!(f.read().unwrap().open_todos("contr").is_empty());
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_794_000_000 + 3_661), "2026-11-06T22:21:01Z");
    }
}
