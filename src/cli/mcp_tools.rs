//! The tools of `contr-rune mcp`: the agent tools `spawn`, `message`, `interrupt`, `stop`, `read`,
//! `prompt`, `list` and `clear_ended`, and the owner tools `ask_owner`, `decided`, `pin_for_owner`,
//! `unpin_for_owner` and `owner_items`. Names, descriptions, argument names and result texts are Go's
//! (go:cmd/contr/mcp.go `agentTools`, `ownerTools`), so the kit's playbook stays valid. The work is in
//! `crate::agents` and `crate::hc::owner`.

use serde::Deserialize;
use serde_json::{Value, json};

use super::agents::{Env, connect, load, spawn_task};
use super::mcp::ToolDef;
use crate::agents::{self, Spec};
use crate::hc::conn::Conn;
use crate::hc::dial::Keep;
use crate::hc::owner::{NewQuestion, NewTodo, OwnerFile};

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
    question: String,
    options: Vec<String>,
    recommended: Option<i64>,
    impacts: Vec<String>,
    refs: Vec<String>,
    source: String,
    title: String,
    from: String,
    kind: String,
    ids: Vec<String>,
    answer: String,
    by: String,
}

impl Args {
    fn parse(raw: Value) -> Result<Args, String> {
        serde_json::from_value(raw).map_err(|e| format!("bad arguments: {e}"))
    }
}

fn schema(required: &[&str], props: &[(&str, &str, &str)]) -> Value {
    let prop = |ty: &str, d: &str| if ty == "string[]" { json!({"type": "array", "items": {"type": "string"}, "description": d}) } else { json!({"type": ty, "description": d}) };
    let props: serde_json::Map<String, Value> = props.iter().map(|(n, ty, d)| ((*n).to_owned(), prop(ty, d))).collect();
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
        ToolDef {
            name: "ask_owner",
            description: "Ask the owner a question only they can answer (a product rule, a design choice, an approval). It is kept until it is decided. Returns its id (D1, D2, …). contr shows it in the questions card, and sends the answer to you as `Owner answered <ID>: <answer>`. When the owner answers in chat, close it with decided.",
            schema: schema(
                &["question"],
                &[
                    ("question", "string", "The question, with enough context to answer it from there alone."),
                    ("options", "string[]", "Short answers to pick from (2-4)."),
                    ("recommended", "integer", "Index in options of your recommendation."),
                    ("title", "string", "A 2-4 word name."),
                    ("from", "string", "The worker, issue or PR it is about."),
                    ("impacts", "string[]", "One line per option, in the order of options: what choosing it means. The sheet shows each under its option."),
                    ("refs", "string[]", "What helps the owner decide: `PR 12` (opens the PR pane), `#12` or `issue 12` (opens in the browser), an http or https URL, or a file path. At most a few."),
                    ("source", "string", "A short excerpt (a few lines) of the text that raised the question, such as a worker's report."),
                ],
            ),
        },
        ToolDef {
            name: "decided",
            description: "Close questions: ones the owner answered in chat (by owner), or ones you settled yourself because they are not the owner's (by coordinator).",
            schema: schema(&["ids", "answer", "by"], &[("ids", "string[]", "Question ids, such as D3."), ("answer", "string", "The answer, short."), ("by", "string", "owner or coordinator.")]),
        },
        ToolDef {
            name: "pin_for_owner",
            description: "Pin one line for the owner: something to do (kind do: test a build, merge a PR that waits for them) or to know (kind know). Returns its id (T1, T2, …).",
            schema: schema(
                &["text"],
                &[("text", "string", "One short line with all it takes to act."), ("kind", "string", "do (the default) or know."), ("from", "string", "The worker, issue or PR it is about.")],
            ),
        },
        ToolDef {
            name: "unpin_for_owner",
            description: "Remove pinned items once they are done or no longer needed.",
            schema: schema(&["ids"], &[("ids", "string[]", "Ids such as T3.")]),
        },
        ToolDef {
            name: "owner_items",
            description: "List the open questions and pinned items of this project (all projects with all).",
            schema: schema(&[], &[("all", "boolean", "All projects.")]),
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
            "ask_owner" | "decided" | "pin_for_owner" | "unpin_for_owner" | "owner_items" => self.owner(name, a),
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// The owner tools write `owner.json` for this project, with this terminal as the coordinator.
    fn owner(&self, name: &str, a: Args) -> Result<String, String> {
        let store = self.env.owner_file.clone().map(OwnerFile::new).ok_or("no home folder to find owner.json in")?;
        let (project, coordinator) = (self.env.project.clone(), self.env.terminal_id.clone());
        let io = |e: std::io::Error| e.to_string();
        match name {
            "ask_owner" => {
                let recommended = a.recommended.map(|r| usize::try_from(r).map_err(|_| format!("recommended {r} is not an option index"))).transpose()?;
                let q = NewQuestion { project, coordinator, from: a.from, title: a.title, question: a.question, options: a.options, recommended, impacts: a.impacts, refs: a.refs, source: a.source };
                store.ask(q).map_err(io)
            }
            "decided" => store.decide(&a.ids, &a.answer, &a.by).map(|()| "decided".into()).map_err(io),
            "pin_for_owner" => store.pin(NewTodo { project, coordinator, from: a.from, kind: a.kind, text: a.text }).map_err(io),
            "unpin_for_owner" => store.unpin(&a.ids).map(|()| "unpinned".into()).map_err(io),
            _ => {
                let (questions, todos) = store.open_items((!a.all).then_some(project.as_str())).map_err(io)?;
                // Go encodes an empty list as null.
                let list = |v: Vec<Value>| if v.is_empty() { Value::Null } else { Value::Array(v) };
                json_text(&json!({"questions": list(questions), "todos": list(todos)}))
            }
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
        Tools::new(Env { config_path: Some(dir.join("agents.json")), kit: None, home: None, terminal_id: String::new(), owner_file: Some(dir.join("owner.json")), project: "proj".into() })
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
            ("ask_owner", json!(["question"])),
            ("decided", json!(["ids", "answer", "by"])),
            ("pin_for_owner", json!(["text"])),
            ("unpin_for_owner", json!(["ids"])),
            ("owner_items", Value::Null),
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
    fn the_owner_tools_refuse_bad_arguments_and_write_nothing() {
        let dir = std::env::temp_dir().join(format!("contr-rune-mcp-owner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = tools(&dir);
        let cases = [
            ("ask_owner", json!({"question": ""}), "a question needs text"),
            ("ask_owner", json!({"question": "q", "options": ["a"], "recommended": 1}), "recommended 1 is not an option index"),
            ("ask_owner", json!({"question": "q", "options": ["a"], "recommended": -1}), "recommended -1 is not an option index"),
            ("ask_owner", json!({"question": "q", "option": ["a"]}), "bad arguments: unknown field `option`"),
            ("ask_owner", json!({"question": "q", "options": "a"}), "bad arguments: invalid type"),
            ("decided", json!({"ids": [], "answer": "x", "by": "owner"}), "no question ids"),
            ("decided", json!({"ids": ["D1"], "answer": "x", "by": "me"}), "by \"me\" is not owner or coordinator"),
            ("decided", json!({"ids": ["D9"], "answer": "x", "by": "owner"}), "no open question D9"),
            ("pin_for_owner", json!({"text": ""}), "a to-do needs text"),
            ("pin_for_owner", json!({"text": "x", "kind": "maybe"}), "kind \"maybe\" is not do or know"),
            ("unpin_for_owner", json!({"ids": []}), "no to-do ids"),
            ("unpin_for_owner", json!({"ids": ["T9"]}), "no open to-do T9"),
        ];
        for (tool, args, want) in cases {
            let e = t.run(tool, args.clone()).unwrap_err();
            assert!(e.starts_with(want), "{tool} {args}: {e}");
        }
        assert!(!dir.join("owner.json").exists());
        assert_eq!(t.run("owner_items", json!({})).unwrap(), "{\n  \"questions\": null,\n  \"todos\": null\n}");
        std::fs::remove_dir_all(dir).unwrap();
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
