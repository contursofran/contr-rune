//! `contr-rune hook post-tool` and `hook prompt` (PARITY-PLAN D7; Go: cmd/contr/hooksession.go).
//! The agent's todos and artifacts, one session file per hc terminal (K2). Both hooks always
//! exit 0: they never block Claude Code. Errors go to stderr.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use super::hook::{Env, read_input};
use crate::paths;
use crate::session_file::{self, Artifact, PENDING, Session, Todo, YOU_PREFIX};

/// Starts the tool names of the artifacts MCP server.
const ARTIFACTS_PREFIX: &str = "mcp__claude-artifacts__";

/// The artifacts tools (name after `ARTIFACTS_PREFIX`) that record an artifact.
const ARTIFACT_TOOLS: [&str; 8] =
    ["publish_version", "publish_files", "edit_version", "read_artifact", "get_artifact", "create_from_type", "read_file", "list_files"];

/// The deepest level `find_artifact` looks into.
const MAX_DEPTH: usize = 6;

fn you_context() -> String {
    format!("contr: before you start, add each new request in this prompt to your todo list (TodoWrite or TaskCreate). Start the text of each such item with \"{YOU_PREFIX}\".")
}

/// `hook post-tool` or `hook prompt`. Always 0.
pub fn run(name: &str, env: &Env, stdin: impl Read + Send + 'static, out: &mut impl Write, err: &mut impl Write) -> i32 {
    // Outside hc there is no terminal to keep a file for.
    if env.terminal_id.is_empty() {
        return 0;
    }
    let result = if name == "prompt" {
        // A worker's prompts come from the coordinator, not the owner.
        if env.coordinator.is_empty() { prompt(stdin, out) } else { Ok(()) }
    } else {
        post_tool(stdin, &env.terminal_id, now())
    };
    if let Err(e) = result {
        let _ = writeln!(err, "contr-rune hook {name}: {e}");
    }
    0
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn prompt(stdin: impl Read + Send + 'static, out: &mut impl Write) -> Result<(), String> {
    let input = read_input(stdin).map_err(|e| format!("bad hook input: {e}"))?;
    let text = input["prompt"].as_str().unwrap_or_default().trim();
    if text.is_empty() || text.starts_with("<hc from=") {
        return Ok(());
    }
    let reply = json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": you_context()}});
    writeln!(out, "{reply}").map_err(|e| e.to_string())
}

fn post_tool(stdin: impl Read + Send + 'static, id: &str, now: u64) -> Result<(), String> {
    let input = read_input(stdin).map_err(|e| format!("bad hook input: {e}"))?;
    let path = session_path(id).ok_or("no terminal id or state dir")?;
    // A bad file is replaced, not kept forever.
    let mut session = session_file::read(&path).unwrap_or_default();
    if !apply(&mut session, &input, now)? {
        return Ok(());
    }
    session.updated_at = now;
    session_file::write(&path, &mut session).map_err(|e| e.to_string())
}

/// The file of terminal `id`, or `None` when the state dir is unknown or the id has no safe
/// characters.
fn session_path(id: &str) -> Option<PathBuf> {
    if paths::sanitize_id(id).trim_matches('_').is_empty() {
        return None;
    }
    paths::session_file(id)
}

/// Applies one PostToolUse input to `session`. `Ok(true)` when it changed.
fn apply(session: &mut Session, input: &Value, now: u64) -> Result<bool, String> {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let (tool_input, response) = (&input["tool_input"], &input["tool_response"]);
    match tool {
        "TodoWrite" => todo_write(session, tool_input),
        "TaskCreate" => task_create(session, tool_input, response),
        "TaskUpdate" => task_update(session, tool_input),
        _ => match tool.strip_prefix(ARTIFACTS_PREFIX) {
            Some(rest) if ARTIFACT_TOOLS.contains(&rest) => Ok(artifact_call(session, tool_input, response, now)),
            _ => Ok(false),
        },
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TodoItem {
    content: String,
    status: String,
    #[serde(rename = "activeForm")]
    active_form: String,
}

fn todo_write(session: &mut Session, input: &Value) -> Result<bool, String> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct In {
        todos: Option<Vec<TodoItem>>,
    }
    let parsed: In = serde_json::from_value(input.clone()).map_err(|e| format!("TodoWrite input: {e}"))?;
    session.todos = parsed.todos.unwrap_or_default().iter().map(|t| Todo::new(&t.content, &t.active_form, &t.status)).collect();
    Ok(true)
}

/// The `TaskCreate` and `TaskUpdate` input fields that are read.
#[derive(Deserialize, Default)]
#[serde(default)]
struct TaskFields {
    #[serde(rename = "taskId")]
    task_id: String,
    subject: String,
    #[serde(rename = "activeForm")]
    active_form: String,
    status: String,
}

fn task_create(session: &mut Session, input: &Value, response: &Value) -> Result<bool, String> {
    let fields: TaskFields = serde_json::from_value(input.clone()).map_err(|e| format!("TaskCreate input: {e}"))?;
    let status = if fields.status.is_empty() { PENDING } else { &fields.status };
    let mut todo = Todo::new(&fields.subject, &fields.active_form, status);
    // The id is optional.
    todo.id = response["task"]["id"].as_str().unwrap_or_default().to_owned();
    if !todo.id.is_empty() {
        if let Some(slot) = session.todos.iter_mut().find(|t| t.id == todo.id) {
            *slot = todo;
            return Ok(true);
        }
    }
    session.todos.push(todo);
    Ok(true)
}

fn task_update(session: &mut Session, input: &Value) -> Result<bool, String> {
    let fields: TaskFields = serde_json::from_value(input.clone()).map_err(|e| format!("TaskUpdate input: {e}"))?;
    if fields.task_id.is_empty() {
        return Ok(false);
    }
    let Some(at) = session.todos.iter().position(|t| t.id == fields.task_id) else {
        return Ok(false);
    };
    if fields.status == "deleted" {
        session.todos.remove(at);
        return Ok(true);
    }
    let next = Todo::new(&fields.subject, &fields.active_form, &fields.status);
    let todo = &mut session.todos[at];
    if !fields.subject.is_empty() {
        todo.text = next.text;
    }
    if !fields.active_form.is_empty() {
        todo.active = next.active;
    }
    if !fields.status.is_empty() {
        todo.status = fields.status;
    }
    todo.you |= next.you;
    Ok(true)
}

/// Records the slug, title and URL found in the input and the response of an artifacts tool
/// call. A call with no slug changes nothing.
fn artifact_call(session: &mut Session, input: &Value, response: &Value, now: u64) -> bool {
    let mut art = Artifact { updated_at: now, ..Artifact::default() };
    for part in [input, response] {
        find_artifact(part, &mut art, 0);
    }
    if art.slug.is_empty() {
        return false;
    }
    if art.url.is_empty() && art.slug.starts_with("https://") {
        // The slug input may be a claude.ai link.
        art.url = art.slug.clone();
    }
    session.add_artifact(art);
    true
}

/// Fills the empty fields of `art` from the first `slug`, `title` and `url` keys in `v`. MCP text
/// content is read too: the first JSON value at the start of a text is decoded and the text after
/// it is ignored.
fn find_artifact(v: &Value, art: &mut Artifact, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            for (key, dst) in [("slug", &mut art.slug), ("title", &mut art.title), ("url", &mut art.url)] {
                if let Some(s) = map.get(key).and_then(Value::as_str) {
                    if dst.is_empty() {
                        *dst = s.to_owned();
                    }
                }
            }
            for sub in map.values() {
                find_artifact(sub, art, depth + 1);
            }
        }
        Value::Array(items) => items.iter().for_each(|sub| find_artifact(sub, art, depth + 1)),
        Value::String(s) => {
            let text = s.trim();
            if text.starts_with('{') {
                if let Some(Ok(inner)) = serde_json::Deserializer::from_str(text).into_iter::<Value>().next() {
                    find_artifact(&inner, art, depth + 1);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(session: &mut Session, tool: &str, input: Value, response: Value) -> Result<bool, String> {
        apply(session, &json!({"tool_name": tool, "tool_input": input, "tool_response": response}), 100)
    }

    fn todo(id: &str, text: &str, status: &str) -> Todo {
        Todo { id: id.into(), text: text.into(), status: status.into(), ..Todo::default() }
    }

    #[test]
    fn todo_write_replaces_the_list_and_cuts_the_you_prefix() {
        let mut s = Session { todos: vec![todo("1", "old", "pending")], ..Session::default() };
        let input = json!({"todos": [
            {"content": "[you] ship it", "status": "in_progress", "activeForm": "Shipping it"},
            {"content": "test", "status": "pending", "activeForm": "Testing"}]});
        assert!(call(&mut s, "TodoWrite", input, Value::Null).unwrap());
        assert_eq!(s.todos.len(), 2);
        assert_eq!((s.todos[0].text.as_str(), s.todos[0].active.as_str(), s.todos[0].you), ("ship it", "Shipping it", true));
        assert_eq!(s.todos[1].status, "pending");
        assert!(call(&mut s, "TodoWrite", json!({"todos": []}), Value::Null).unwrap());
        assert!(s.todos.is_empty());
        assert!(call(&mut s, "TodoWrite", json!("nope"), Value::Null).unwrap_err().starts_with("TodoWrite input"));
    }

    #[test]
    fn task_create_appends_or_replaces_by_response_id() {
        let mut s = Session::default();
        let resp = json!({"task": {"id": "4"}});
        assert!(call(&mut s, "TaskCreate", json!({"subject": "[you] a", "activeForm": "Doing a"}), resp.clone()).unwrap());
        assert_eq!(s.todos, vec![Todo { id: "4".into(), text: "a".into(), active: "Doing a".into(), status: "pending".into(), you: true }]);
        call(&mut s, "TaskCreate", json!({"subject": "b", "status": "in_progress"}), resp).unwrap();
        assert_eq!((s.todos.len(), s.todos[0].text.as_str(), s.todos[0].status.as_str()), (1, "b", "in_progress"));
        call(&mut s, "TaskCreate", json!({"subject": "c"}), json!("no id here")).unwrap();
        assert_eq!((s.todos.len(), s.todos[1].id.as_str()), (2, ""));
        assert!(call(&mut s, "TaskCreate", json!(5), Value::Null).is_err());
    }

    #[test]
    fn task_update_table() {
        let start = || Session { todos: vec![Todo { you: false, ..todo("1", "one", "pending") }, todo("2", "two", "pending")], ..Session::default() };
        // (input, changed, resulting todos as "id:text:status:you")
        let cases = [
            (json!({"taskId": "1", "status": "completed"}), true, vec!["1:one:completed:false", "2:two:pending:false"]),
            (json!({"taskId": "2", "subject": "[you] deux", "activeForm": "Doing"}), true, vec!["1:one:pending:false", "2:deux:pending:true"]),
            (json!({"taskId": "1", "status": "deleted"}), true, vec!["2:two:pending:false"]),
            (json!({"taskId": "9", "status": "completed"}), false, vec!["1:one:pending:false", "2:two:pending:false"]),
            (json!({"status": "completed"}), false, vec!["1:one:pending:false", "2:two:pending:false"]),
        ];
        for (input, changed, want) in cases {
            let mut s = start();
            assert_eq!(call(&mut s, "TaskUpdate", input.clone(), Value::Null).unwrap(), changed, "{input}");
            let got: Vec<String> = s.todos.iter().map(|t| format!("{}:{}:{}:{}", t.id, t.text, t.status, t.you)).collect();
            assert_eq!(got, want, "{input}");
        }
        assert!(call(&mut start(), "TaskUpdate", json!("x"), Value::Null).is_err());
    }

    #[test]
    fn artifact_calls_move_the_slug_to_the_top() {
        let mut s = Session::default();
        let tool = |n: &str| format!("mcp__claude-artifacts__{n}");
        // Slug in the input, title and url in the response's text content.
        let resp = json!({"content": [{"type": "text", "text": "{\"title\":\"Plan\",\"url\":\"https://claude.ai/a/x\"} trailing words"}]});
        assert!(call(&mut s, &tool("publish_version"), json!({"slug": "x"}), resp).unwrap());
        assert!(call(&mut s, &tool("read_artifact"), json!({"slug": "y"}), json!({"title": "Y"})).unwrap());
        assert!(call(&mut s, &tool("get_artifact"), json!({"slug": "x"}), json!("plain")).unwrap());
        let got: Vec<_> = s.artifacts.iter().map(|a| (a.slug.as_str(), a.title.as_str(), a.url.as_str(), a.updated_at)).collect();
        assert_eq!(got, vec![("x", "Plan", "https://claude.ai/a/x", 100), ("y", "Y", "", 100)]);
        // A link as slug is its own url.
        call(&mut s, &tool("edit_version"), json!({"slug": "https://claude.ai/a/z"}), Value::Null).unwrap();
        assert_eq!(s.artifacts[0].url, "https://claude.ai/a/z");
        // No slug, a tool that records nothing, and other tools change nothing.
        assert!(!call(&mut s, &tool("read_file"), json!({"path": "a"}), json!({"title": "T"})).unwrap());
        assert!(!call(&mut s, &tool("guide"), json!({"slug": "q"}), Value::Null).unwrap());
        assert!(!call(&mut s, "Bash", json!({"slug": "q"}), Value::Null).unwrap());
        assert!(!call(&mut s, "claude-artifacts__publish_version", json!({"slug": "q"}), Value::Null).unwrap());
        assert_eq!(s.artifacts.len(), 3);
    }

    #[test]
    fn artifact_search_stops_at_depth_6() {
        let deep = |n: usize| (0..n).fold(json!({"slug": "s"}), |inner, _| json!({"a": inner}));
        let mut art = Artifact::default();
        find_artifact(&deep(6), &mut art, 0);
        assert_eq!(art.slug, "s");
        let mut art = Artifact::default();
        find_artifact(&deep(7), &mut art, 0);
        assert_eq!(art.slug, "");
    }

    #[test]
    fn prompt_asks_for_you_todos_except_hc_prompts_workers_and_empty_prompts() {
        let ask = |terminal: &str, coordinator: &str, prompt: &str| {
            let env = Env { coordinator: coordinator.into(), terminal_id: terminal.into(), effort: String::new(), compact_window: String::new(), restore: super::super::restore::Env::default() };
            let input = json!({"prompt": prompt}).to_string();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(run("prompt", &env, std::io::Cursor::new(input.into_bytes()), &mut out, &mut err), 0);
            String::from_utf8(out).unwrap()
        };
        let out: Value = serde_json::from_str(&ask("t1", "", "fix the bug")).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(out["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("\"[you] \""));
        assert_eq!(ask("t1", "", "<hc from=\"t3\">do it</hc>"), "");
        assert_eq!(ask("t1", "", "   "), "");
        assert_eq!(ask("t1", "t3", "fix the bug"), "");
        assert_eq!(ask("", "", "fix the bug"), "");
    }
}
