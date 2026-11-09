//! The standalone `contr-rune <command>` CLI. It runs before Rune starts, so a CLI call never
//! opens a window. Exit codes: 0 ok, 1 error, 2 usage (PARITY-PLAN K4).

mod agents;
mod hook;
mod hook_session;
mod mcp;
mod mcp_tools;
mod owner;
mod restore;
mod statusline;
mod turn;

use std::ffi::OsString;

const USAGE: &str = "\
usage: contr-rune <command>

commands:
  agents       spawn and manage agent terminals
  owner        questions for the owner
  hook         Claude Code hooks (stop, session-start, post-tool, prompt)
  mcp          MCP server on stdio
  statusline   Claude Code status line
  help         show this text

With no command, contr-rune opens the app.";

/// The commands that are still stubs; each part replaces its entry.
const STUBS: [&str; 0] = [];

/// Runs a CLI call and returns its exit code, or `None` when the arguments are not a CLI call
/// (no argument, or the app's own `--` flags): then the app starts.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let args: Vec<String> = args.into_iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let first = args.first()?;
    if first.starts_with('-') {
        return None;
    }
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    Some(dispatch(&args, &mut out, &mut err))
}

fn dispatch(args: &[String], out: &mut impl std::io::Write, err: &mut impl std::io::Write) -> i32 {
    match args[0].as_str() {
        "agents" => agents::run(&args[1..], &agents::Env::real(), out, err),
        "owner" => owner::run(&args[1..], &owner::Env::real(), out, err),
        "hook" => hook::run(&args[1..], &hook::Env::real(), std::io::stdin(), out, err),
        "mcp" => mcp_tools::run(std::io::stdin().lock(), out, err),
        "statusline" => statusline::run(&args[1..], &statusline::Env::real(), std::io::stdin().lock(), out, err),
        "help" => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        c if STUBS.contains(&c) => {
            let _ = writeln!(err, "contr-rune {c}: not built yet");
            2
        }
        other => {
            let _ = writeln!(err, "contr-rune: unknown command `{other}`\n\n{USAGE}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        std::iter::once("contr-rune").chain(list.iter().copied()).map(OsString::from).collect()
    }

    fn call(cmd: &str) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = dispatch(&[cmd.to_owned()], &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn the_app_starts_without_a_command_or_with_its_own_flags() {
        assert_eq!(run(args(&[])), None);
        assert_eq!(run(args(&["--snap", "a.png"])), None);
    }

    #[test]
    fn help_prints_the_usage_and_exits_0() {
        let (code, out, err) = call("help");
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(out.starts_with("usage: contr-rune"));
    }

    #[test]
    fn a_stub_says_not_built_yet_and_an_unknown_word_is_a_usage_error() {
        for cmd in STUBS {
            let (code, out, err) = call(cmd);
            assert_eq!((code, out.as_str()), (2, ""));
            assert_eq!(err, format!("contr-rune {cmd}: not built yet\n"));
        }
        let (code, _, err) = call("bogus");
        assert_eq!(code, 2);
        assert!(err.contains("unknown command `bogus`"));
    }
}
