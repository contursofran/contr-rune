//! `contr-rune owner ask | answer | done` (PARITY-PLAN K4; Go: cmd/contr/owner.go). Exit codes: 0
//! ok, 1 error, 2 usage. `answer` and `done` do what the questions card does: the owner file
//! changes first, then the coordinator that asked is told; when that fails the change stays saved
//! and the exit code is 1.

use std::io::Write;
use std::path::PathBuf;

use crate::agents;
use crate::hc::owner::{NewQuestion, OwnerFile, OwnerSource};
use crate::paths;

const USAGE: &str = "\
Usage:
  contr-rune owner ask [flags] <question…> ask a question; prints its id (D1, …)
  contr-rune owner answer <id> <answer…>   answer a question (D1, …) and tell its coordinator
  contr-rune owner done <id>               mark a to-do (T1, …) done and tell its coordinator

Flags of ask (each of option, impact and ref can repeat):
  --title T         a 2-4 word name
  --from F          the worker, issue or PR it is about
  --option O        a short answer to pick from
  --impact I        what choosing the option means; the nth impact is for the nth option
  --recommended N   index (from 0) of the recommended option
  --ref R           what helps to decide: \"PR 12\", \"#12\" or \"issue 12\", an http(s) URL, a file path
  --source S        a short excerpt of the text that raised the question

answer and done do what the questions card does: the owner file changes first, then the
coordinator that asked gets \"Owner answered <id>: <answer>\" or
\"Owner did <id>: <text>\". When the coordinator cannot be reached the change
stays saved and the command exits 1.

Environment:
  CONTR_OWNER_FILE   the owner file (default: ~/Library/Application Support/contr/owner.json)
  CONTR_PROJECT      the project of a question (default: the name of the current folder)
  CONTR_HC           the hc program
  HC_TERMINAL_ID     set inside hc: ask makes that terminal the coordinator
";

/// The environment a call reads, so tests need not touch the process environment.
pub struct Env {
    pub owner_file: Option<PathBuf>,
    pub project: String,
    pub terminal_id: String,
}

impl Env {
    pub fn real() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(|v| v.to_string_lossy().into_owned());
        let project = var("CONTR_PROJECT").or_else(|| {
            let cwd = std::env::current_dir().ok()?;
            cwd.file_name().map(|n| n.to_string_lossy().into_owned())
        });
        Env { owner_file: paths::owner_file(), project: project.unwrap_or_default(), terminal_id: var("HC_TERMINAL_ID").unwrap_or_default() }
    }
}

/// Runs `contr-rune owner <args>` and returns the exit code.
pub fn run(args: &[String], env: &Env, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let Some(sub) = args.first().map(String::as_str) else {
        let _ = err.write_all(USAGE.as_bytes());
        return 2;
    };
    let rest = &args[1..];
    match sub {
        "help" | "-h" | "--help" => {
            let _ = out.write_all(USAGE.as_bytes());
            0
        }
        "ask" => ask(rest, env, out, err),
        "answer" | "done" => {
            let right = match sub {
                "answer" => rest.len() >= 2 && !rest[1..].join(" ").trim().is_empty(),
                _ => rest.len() == 1,
            };
            if !right {
                let _ = write!(err, "contr-rune owner {sub}: wrong number of arguments\n{USAGE}");
                return 2;
            }
            change(sub, &rest[0], &rest[1..].join(" "), env, err)
        }
        other => {
            let _ = write!(err, "contr-rune owner: unknown command {other}\n{USAGE}");
            2
        }
    }
}

fn fail(err: &mut impl Write, msg: &str) -> i32 {
    let _ = writeln!(err, "{msg}");
    1
}

fn store(env: &Env) -> Result<OwnerFile, String> {
    env.owner_file.clone().map(OwnerFile::new).ok_or_else(|| "contr-rune owner: no home folder to find owner.json in".to_owned())
}

/// The flags of `ask`, parsed as Go's `flag` package does: `-x` or `--x`, `--x=v` or `--x v`,
/// they end at the first other word or at `--`, and every later word is part of the question.
#[derive(Debug, Default, PartialEq)]
struct AskArgs {
    title: String,
    from: String,
    source: String,
    recommended: Option<usize>,
    options: Vec<String>,
    impacts: Vec<String>,
    refs: Vec<String>,
    question: String,
}

fn parse_ask(args: &[String]) -> Result<AskArgs, String> {
    let mut a = AskArgs::default();
    let mut recommended = -1_i64;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.peek().filter(|a| a.starts_with('-') && a.len() > 1) {
        let arg = arg.as_str();
        it.next();
        if arg == "--" {
            break;
        }
        let body = arg.trim_start_matches('-');
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (body, None),
        };
        if !matches!(name, "title" | "from" | "source" | "recommended" | "option" | "impact" | "ref") {
            return Err(format!("flag provided but not defined: -{name}"));
        }
        let value = match inline {
            Some(v) => v,
            None => it.next().cloned().ok_or_else(|| format!("flag needs an argument: -{name}"))?,
        };
        match name {
            "title" => a.title = value,
            "from" => a.from = value,
            "source" => a.source = value,
            "option" => a.options.push(value),
            "impact" => a.impacts.push(value),
            "ref" => a.refs.push(value),
            _ => recommended = value.parse().map_err(|_| format!("invalid value {value:?} for flag -recommended: parse error"))?,
        }
    }
    a.question = it.cloned().collect::<Vec<_>>().join(" ").trim().to_owned();
    if a.question.is_empty() {
        return Err("give the question".to_owned());
    }
    if usize::try_from(recommended).is_ok_and(|r| r >= a.options.len()) || a.impacts.len() > a.options.len() {
        return Err("--recommended and --impact need a matching --option".to_owned());
    }
    a.recommended = usize::try_from(recommended).ok();
    Ok(a)
}

fn ask(args: &[String], env: &Env, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let a = match parse_ask(args) {
        Ok(a) => a,
        Err(e) => {
            let _ = write!(err, "contr-rune owner ask: {e}\n{USAGE}");
            return 2;
        }
    };
    let file = match store(env) {
        Ok(f) => f,
        Err(e) => return fail(err, &e),
    };
    let q = NewQuestion {
        project: env.project.clone(),
        coordinator: env.terminal_id.clone(),
        from: a.from,
        title: a.title,
        question: a.question,
        options: a.options,
        recommended: a.recommended,
        impacts: a.impacts,
        refs: a.refs,
        source: a.source,
    };
    match file.ask(q) {
        Ok(id) => {
            let _ = writeln!(out, "{id}");
            0
        }
        Err(e) => fail(err, &format!("contr-rune owner ask: {e}")),
    }
}

/// `answer <id> <text>` or `done <id>`: save, then tell the coordinator.
fn change(sub: &str, id: &str, answer: &str, env: &Env, err: &mut impl Write) -> i32 {
    let file = match store(env) {
        Ok(f) => f,
        Err(e) => return fail(err, &e),
    };
    let saved = file.read().and_then(|st| {
        let ids = [id.to_owned()];
        if sub == "answer" {
            let coordinator = st.questions.iter().find(|q| q.id == id).map(|q| q.coordinator.clone()).unwrap_or_default();
            file.decide(&ids, answer, "owner")?;
            Ok((coordinator, format!("Owner answered {id}: {answer}\n")))
        } else {
            let todo = st.todos.iter().find(|t| t.id == id).cloned().unwrap_or_default();
            file.unpin(&ids)?;
            Ok((todo.coordinator, format!("Owner did {id}: {}\n", todo.text)))
        }
    });
    let (coordinator, text) = match saved {
        Ok(s) => s,
        Err(e) => return fail(err, &format!("contr-rune owner {sub}: {e}")),
    };
    if coordinator.is_empty() {
        return 0;
    }
    let told = super::agents::connect().and_then(|(conn, _keep)| agents::send(&conn, "", &coordinator, &text));
    match told {
        Ok(()) => 0,
        Err(e) => fail(err, &format!("contr-rune owner {sub}: saved; coordinator not reached: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ask_flags_come_first_and_the_rest_is_the_question() {
        let a = parse_ask(&words(&["--title", "Ship", "-option=yes", "--option", "no", "--impact", "fast", "--recommended", "1", "--ref", "#12", "--ref", "PR 3", "--source=log", "--from", "b-1", "Ship", "it?", "--title"])).unwrap();
        assert_eq!(
            a,
            AskArgs {
                title: "Ship".into(),
                from: "b-1".into(),
                source: "log".into(),
                recommended: Some(1),
                options: words(&["yes", "no"]),
                impacts: words(&["fast"]),
                refs: words(&["#12", "PR 3"]),
                question: "Ship it? --title".into(),
            },
            "a flag after the first word is part of the question"
        );
        assert_eq!(parse_ask(&words(&["--", "--odd", "word"])).unwrap().question, "--odd word");
    }

    #[test]
    fn ask_mistakes_are_usage_errors() {
        let bad = |list: &[&str]| parse_ask(&words(list)).unwrap_err();
        assert_eq!(bad(&[]), "give the question");
        assert_eq!(bad(&["--title", "T"]), "give the question");
        assert_eq!(bad(&["--bogus", "x", "q"]), "flag provided but not defined: -bogus");
        assert_eq!(bad(&["--title"]), "flag needs an argument: -title");
        assert_eq!(bad(&["--recommended", "x", "q"]), "invalid value \"x\" for flag -recommended: parse error");
        let need = "--recommended and --impact need a matching --option";
        assert_eq!(bad(&["--recommended", "0", "q"]), need, "no option to recommend");
        assert_eq!(bad(&["--option", "a", "--recommended", "1", "q"]), need, "past the last option");
        assert_eq!(bad(&["--option", "a", "--impact", "x", "--impact", "y", "q"]), need, "more impacts than options");
    }
}
