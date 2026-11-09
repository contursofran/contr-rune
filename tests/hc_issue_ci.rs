//! Issue titles and CI on agent rows (A4), against the in-process fake core and a fake `gh`
//! (`CONTR_GH`). Worker t17 (issue 241, working) and t18 (issue 242, needs you, has a `worktree`
//! label) read `gh issue view <N> --json title`; t18's PR has an unfinished check, so its meta
//! line ends with `CI`, and t17's does not while it works. Without `gh` the rows keep their
//! terminal titles. No test runs a real `gh`.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use common::*;
use serde_json::{Value, json};

/// A fake `gh`: `issue view N` answers `{"title":"Issue N"}`, `pr view` answers an open PR with a
/// running check; with `fail` every call exits 1.
fn fake_gh(name: &str, fail: bool) -> PathBuf {
    let script = std::env::temp_dir().join(format!("contr-rune-gh-a4-{}-{name}", std::process::id()));
    let body = if fail {
        "#!/bin/sh\necho no >&2\nexit 1\n".to_owned()
    } else {
        "#!/bin/sh\ncase \"$1\" in\nissue) echo \"{\\\"title\\\":\\\"Issue $3\\\"}\";;\n\
         pr) echo '{\"number\":9,\"state\":\"OPEN\",\"statusCheckRollup\":[{\"status\":\"IN_PROGRESS\",\"conclusion\":\"\"}]}';;\nesac\n"
            .to_owned()
    };
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn run(gh: Option<PathBuf>) -> Run {
    let mut app = App::new().project("contr").scene("agents").fixture_with(|d| {
        let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
        for t in fx["terminals"].as_array_mut().unwrap() {
            t["cwd"] = json!(d.proj);
            if t["address"] == "t18" {
                t["labels"]["worktree"] = json!(d.proj);
            }
        }
        fx
    });
    if let Some(gh) = gh {
        app = app.env("CONTR_GH", gh.to_str().unwrap());
    }
    app.end(2.0).run()
}

#[test]
fn rows_show_the_issue_title_and_ci_for_a_waiting_worker() {
    let r = run(Some(fake_gh("ok", false)));
    assert_eq!(r.text_in("agent/t17", "label").as_deref(), Some("Issue 241"));
    assert_eq!(r.text_in("agent/t18", "label").as_deref(), Some("Issue 242"));
    assert_eq!(r.text_in("agent/t18", "subtitle").as_deref(), Some("b-242  sonnet  CI"));
    assert_eq!(r.text_in("agent/t17", "subtitle").as_deref(), Some("b-241  sonnet"), "a working agent shows no CI");
    assert_eq!(r.text_in("agent/t3", "label").as_deref(), Some("Plan H1"), "no issue: the terminal title stays");
}

#[test]
fn rows_are_unchanged_without_gh_or_when_it_fails() {
    for gh in [None, Some(fake_gh("fail", true))] {
        let r = run(gh);
        assert_eq!(r.text_in("agent/t17", "label").as_deref(), Some("Build H1"));
        assert_eq!(r.text_in("agent/t18", "label").as_deref(), Some("Build H2"));
        assert_eq!(r.text_in("agent/t18", "subtitle").as_deref(), Some("b-242  sonnet"));
    }
}
