//! The CLI runs without a window: `help` and a bad command return at once with the right code.

use std::process::Command;

fn contr(args: &[&str]) -> (i32, String, String) {
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune")).args(args).output().unwrap();
    (
        run.status.code().unwrap(),
        String::from_utf8_lossy(&run.stdout).into_owned(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
    )
}

#[test]
fn help_exits_0_and_a_bad_command_exits_2() {
    let (code, out, _) = contr(&["help"]);
    assert_eq!(code, 0);
    assert!(out.contains("usage: contr-rune"));

    let (code, out, err) = contr(&["bogus"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("unknown command `bogus`"));
}
