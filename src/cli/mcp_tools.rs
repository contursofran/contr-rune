//! The agent tools of `contr-rune mcp`: `spawn`, `message`, `interrupt`, `stop`, `read`, `prompt`,
//! `list` and `clear_ended`. Names, descriptions, argument names and result texts are Go's
//! (go:cmd/contr/mcp.go `agentTools`), so the kit's playbook stays valid. The work is in
//! `crate::agents`.

use serde::Deserialize;
use serde_json::{Value, json};

use super::agents::{Env, connect, load, spawn_task};
use super::mcp::ToolDef;
use crate::agents::{self, Spec};
use crate::hc::conn::Conn;
use crate::hc::dial::Keep;

/// The arguments of every tool, as Go's one `args` struct. An unknown argument is an error, so a
/// misnamed field (`message` for `text`) never becomes an empty value (D200).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Args {
    name: String,
    profile: String,
    task: String,
    task_file: String,
    issue: String,
    worktree: String,
    project: String,
    role: String,
    text: String,
    lines: i64,
    all: bool,
    keep: bool,
    review_pr: i64,
}

impl Args {
    fn parse(raw: Value) -> Result<Args, String> {
        serde_json::from_value(raw).map_err(|e| format!("bad arguments: {e}"))
    }
}

fn schema(required: &[&str], props: &[(&str, &str, &str)]) -> Value {
    let props: serde_json::Map<String, Value> = props.iter().map(|(n, ty, d)| ((*n).to_owned(), json!({"type": ty, "description": d}))).collect();
    let mut s = json!({"type": "object", "properties": props});
    if !required.is_empty() {
        s["required"] = json!(required);
    }
    s
}

pub fn defs() -> Vec<ToolDef> {
    let worker = ("name", "string", "Worker name.");
    vec![
        ToolDef {
            name: "spawn",
            description: "Start a worker agent in a new hc terminal, in its worktree. It reports back with hc send when its turn ends.",
            schema: schema(
                &["name", "profile"],
                &[
                    ("name", "string", "Unique worker name: 1-32 of a-z 0-9 - _ (for example b-158)."),
                    ("profile", "string", "builder, planner, reviewer or debugger, or a profile from the profiles file."),
                    ("task", "string", "The self-contained brief: the first message. Give task or task_file."),
                    ("task_file", "string", "An absolute path to a file that holds the brief, instead of task."),
                    ("issue", "string", "Issue number."),
                    ("worktree", "string", "The worker's working folder (its git worktree)."),
                    ("role", "string", "Role label; the profile name by default."),
                    ("project", "string", "Project name; groups the worker with its project's questions."),
                    ("review_pr", "integer", "A pull request number: start the worker in a detached worktree at the PR's head, <main checkout>-wt/review-<N>, of the repository at worktree (default: the coordinator's folder)."),
                ],
            ),
        },
        ToolDef {
            name: "message",
            description: "Type a message into a worker and press Enter. It keeps the worker's context.",
            schema: schema(&["name", "text"], &[worker, ("text", "string", "The message.")]),
        },
        ToolDef {
            name: "interrupt",
            description: "Press a worker's stop key to end its turn; it keeps its context. Returns hc's outcome.",
            schema: schema(&["name"], &[worker]),
        },
        ToolDef {
            name: "stop",
            description: "End a worker's program and remove its terminal from hc. Its context is lost. Read its RESULT.md first.",
            schema: schema(&["name"], &[worker, ("keep", "boolean", "Keep the finished terminal so it can still be read. Default false.")]),
        },
        ToolDef { name: "read", description: "Read a worker's screen and history.", schema: schema(&["name"], &[worker, ("lines", "integer", "Only the last lines.")]) },
        ToolDef {
            name: "prompt",
            description: "Show the menu a blocked worker waits on: for a permission, the tool and its command, then the question and the numbered choices. \"no prompt\" when it waits on none.",
            schema: schema(&["name"], &[worker]),
        },
        ToolDef { name: "list", description: "List the workers with their role, issue, state and worktree.", schema: schema(&[], &[("all", "boolean", "Include finished workers.")]) },
        ToolDef {
            name: "clear_ended",
            description: "Remove the finished terminals of the workers this coordinator started. Other coordinators' workers and plain terminals stay. Returns the names removed.",
            schema: schema(&[], &[]),
        },
    ]
}

/// `contr-rune mcp`: serves the tools on stdin and stdout until stdin ends.
pub fn run(input: impl std::io::BufRead, out: &mut impl std::io::Write, err: &mut impl std::io::Write) -> i32 {
    let mut tools = Tools::new(Env::real());
    match super::mcp::serve(input, out, "contr", env!("CARGO_PKG_VERSION"), &defs(), &mut |name, args| tools.run(name, args)) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "contr mcp: {e}");
            1
        }
    }
}

/// Runs the tools. It dials the core on the first call that needs it, and again after the
/// connection ended.
pub struct Tools {
    env: Env,
    link: Option<(Conn, Keep)>,
}

impl Tools {
    pub fn new(env: Env) -> Tools {
        Tools { env, link: None }
    }

    fn conn(&mut self) -> Result<&Conn, String> {
        if self.link.as_ref().is_some_and(|(c, _)| c.is_closed()) {
            self.link = None;
        }
        if self.link.is_none() {
            self.link = Some(connect()?);
        }
        Ok(&self.link.as_ref().expect("connected above").0)
    }

    /// Runs tool `name` with `raw` arguments and returns its result text.
    pub fn run(&mut self, name: &str, raw: Value) -> Result<String, String> {
        let a = Args::parse(raw)?;
        let caller = self.env.terminal_id.clone();
        match name {
            "spawn" => self.spawn(a),
            "message" => {
                agents::send(self.conn()?, &caller, &a.name, &a.text)?;
                Ok("sent".into())
            }
            "interrupt" => agents::interrupt(self.conn()?, &caller, &a.name),
            "stop" => {
                agents::stop(self.conn()?, &caller, &a.name, a.keep)?;
                Ok("stopped".into())
            }
            "read" => agents::read(self.conn()?, &caller, &a.name, a.lines),
            "prompt" => agents::pending_prompt(self.conn()?, &caller, &a.name),
            "list" => json_text(&agents::list(self.conn()?, a.all)?),
            "clear_ended" => match agents::remove_ended(self.conn()?, &caller) {
                (_, Some(e)) => Err(e),
                (names, None) => json_text(&names),
            },
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// The profiles file is read again here, so a profile added while the session runs works at once.
    fn spawn(&mut self, a: Args) -> Result<String, String> {
        let cfg = load(&self.env)?;
        let task = spawn_task(&a.task, &a.task_file)?;
        if task.trim().is_empty() {
            return Err("give the brief as task or task_file".into());
        }
        let mut worktree = a.worktree;
        if a.review_pr != 0 {
            worktree = agents::review_worktree(if worktree.is_empty() { "." } else { &worktree }, a.review_pr)?;
        }
        let spec = Spec { name: a.name, profile: a.profile, task, issue: a.issue, worktree, role: a.role, project: a.project, parent: self.env.terminal_id.clone() };
        let (kit, home) = (self.env.kit.clone(), self.env.home.clone());
        let worker = agents::spawn(self.conn()?, &cfg, spec, kit.as_ref(), home.as_deref())?;
        json_text(&worker)
    }
}

/// Indented JSON, as Go's `toJSON`.
fn json_text(v: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(dir: &std::path::Path) -> Tools {
        Tools::new(Env { config_path: Some(dir.join("agents.json")), kit: None, home: None, terminal_id: String::new() })
    }

    #[test]
    fn the_tools_are_the_ones_of_go_with_their_required_arguments() {
        let got: Vec<(&str, Value)> = defs().iter().map(|t| (t.name, t.schema["required"].clone())).collect();
        let want = [
            ("spawn", json!(["name", "profile"])),
            ("message", json!(["name", "text"])),
            ("interrupt", json!(["name"])),
            ("stop", json!(["name"])),
            ("read", json!(["name"])),
            ("prompt", json!(["name"])),
            ("list", Value::Null),
            ("clear_ended", Value::Null),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn an_unknown_or_mistyped_argument_is_refused_before_anything_runs() {
        let dir = std::env::temp_dir();
        let mut t = tools(&dir);
        // `message` for `text`: Go's #483 case.
        let e = t.run("message", json!({"name": "b-1", "message": "hi"})).unwrap_err();
        assert!(e.starts_with("bad arguments: unknown field `message`"), "{e}");
        let e = t.run("read", json!({"name": "b-1", "lines": "ten"})).unwrap_err();
        assert!(e.starts_with("bad arguments: invalid type"), "{e}");
        assert!(t.link.is_none(), "no connection for a refused call");
    }

    #[test]
    fn spawn_wants_exactly_one_brief_and_never_dials_without_it() {
        let dir = std::env::temp_dir().join(format!("contr-rune-mcp-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("brief.md");
        std::fs::write(&file, "do it").unwrap();
        let mut t = tools(&dir);
        let blank = t.run("spawn", json!({"name": "b-1", "profile": "builder", "task": "  "})).unwrap_err();
        assert_eq!(blank, "give the brief as task or task_file");
        let both = t.run("spawn", json!({"name": "b-1", "profile": "builder", "task": "x", "task_file": file})).unwrap_err();
        assert_eq!(both, "give the task as text or with a task file, not both");
        let missing = t.run("spawn", json!({"name": "b-1", "profile": "builder", "task_file": dir.join("none.md")})).unwrap_err();
        assert!(missing.starts_with("task file: "), "{missing}");
        assert!(t.link.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spawn_reads_the_profiles_file_on_each_call() {
        let dir = std::env::temp_dir().join(format!("contr-rune-mcp-profiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = tools(&dir);
        std::fs::write(dir.join("agents.json"), "{not json").unwrap();
        let e = t.run("spawn", json!({"name": "b-1", "profile": "builder", "task": "x"})).unwrap_err();
        assert!(e.starts_with("agents: ") && e.contains("agents.json"), "{e}");
        std::fs::write(dir.join("agents.json"), "{}").unwrap();
        let e = t.run("spawn", json!({"name": "b-1", "profile": "builder", "task": " "})).unwrap_err();
        assert_eq!(e, "give the brief as task or task_file");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
