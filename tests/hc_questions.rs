//! The questions notch and card (H9), against the in-process fake core (`CONTR_HC_FAKE`, the fixture
//! `tests/fixtures/hc/agents.json`, log in `CONTR_HC_FAKE_LOG`) and a temp copy of
//! `tests/fixtures/hc/owner.json` (`CONTR_OWNER_FILE`). The project `contr` has the blocked agent
//! t18, the questions D7 (options, the second one recommended) and D8 (no options) and the to-do T4,
//! so the notch reads "4 items" and the card has the tabs `b-242`, `D7`, `D8`, `T4`. D9 belongs to
//! another project. No test dials a real hc or reads the owner's real file.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Run {
    tree: Value,
    log: Vec<Value>,
    owner: Value,
    dir: PathBuf,
}

fn tap(t: f64, key: &str) -> Value {
    json!({ "t": t, "type": "tap", "key": key })
}
fn typed(t: f64, text: &str) -> Value {
    json!({ "t": t, "type": "type", "text": text })
}
fn key(t: f64, name: &str) -> Value {
    json!({ "t": t, "type": "key", "name": name })
}

/// ⌘ + `name` on macOS, Ctrl+Shift + `name` elsewhere.
fn chord(t: f64, name: &str) -> Value {
    let mac = cfg!(target_os = "macos");
    json!({ "t": t, "type": "key", "name": name, "cmd": true, "shift": !mac })
}

/// The card's tabs: `segment<i>` for `b-242`, `D7`, `D8`, `T4`.
const TAB_D7: &str = "segment1";
const TAB_D8: &str = "segment2";
const TAB_T4: &str = "segment3";

/// Run with `events`; `blocked_send` keeps the fixture's `terminal.send` error (`blocked`), `quiet`
/// makes the blocked agent idle and the owner's file empty.
fn run_with(events: Vec<Value>, blocked_send: bool, quiet: bool) -> Run {
    let mut events = events;
    events.push(json!({ "t": 9.0, "type": "mark", "name": "end" }));
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = format!(
        "{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(format!("contr-rune-hcquestions-{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("contr");
    std::fs::create_dir_all(&proj).unwrap();
    let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
    for t in fx["terminals"].as_array_mut().unwrap() {
        t["cwd"] = json!(proj);
        if quiet && t["address"] == "t18" {
            t["status"]["state"] = json!("idle");
        }
    }
    if !blocked_send {
        fx["errors"] = json!({});
    }
    let owner_text = if quiet {
        json!({"questions": [], "todos": []}).to_string()
    } else {
        include_str!("fixtures/hc/owner.json").to_owned()
    };
    let (script, out, log, fixture, owner) = (
        dir.join("script.json"),
        dir.join("tree.json"),
        dir.join("log.jsonl"),
        dir.join("fixture.json"),
        dir.join("owner.json"),
    );
    std::fs::write(&script, json!({ "events": events }).to_string()).unwrap();
    std::fs::write(&fixture, fx.to_string()).unwrap();
    std::fs::write(&owner, owner_text).unwrap();
    let r = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env_remove("CONTR_HC")
        .env("CONTR_HC_FAKE", &fixture)
        .env("CONTR_HC_FAKE_LOG", &log)
        .env("CONTR_OWNER_FILE", &owner)
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .env("CONTR_PROJECTS", &proj)
        .args([
            "--script",
            script.to_str().unwrap(),
            "--tree",
            out.to_str().unwrap(),
            "--size",
            "1710x1112",
            "--scale",
            "1",
            "--at",
            "mark:end",
        ])
        .output()
        .expect("run the app");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let tree = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let log = std::fs::read_to_string(&log)
        .map(|t| {
            t.lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        })
        .unwrap_or_default();
    let owner = serde_json::from_slice(&std::fs::read(&owner).unwrap()).unwrap();
    Run {
        tree,
        log,
        owner,
        dir,
    }
}

fn run(events: Vec<Value>) -> Run {
    run_with(events, false, false)
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Run {
    fn find(&self, key: &str) -> Option<&Value> {
        self.tree["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
    }

    fn text(&self, key: &str) -> String {
        self.find(key).unwrap_or_else(|| panic!("no element {key}"))["text"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    fn label(&self) -> Option<String> {
        self.find("notch/label").map(|_| self.text("notch/label"))
    }

    fn card_open(&self) -> bool {
        self.find("qcard").is_some()
    }

    fn sends(&self) -> Vec<&Value> {
        self.log
            .iter()
            .filter(|l| l["method"] == "terminal.send")
            .map(|l| &l["params"])
            .collect()
    }

    fn question(&self, id: &str) -> &Value {
        self.owner["questions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == id)
            .unwrap()
    }

    fn todo(&self, id: &str) -> &Value {
        self.owner["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .unwrap()
    }
}

#[test]
fn the_notch_counts_the_projects_open_items_and_the_card_starts_closed() {
    let r = run(vec![]);
    assert_eq!(
        r.label().as_deref(),
        Some("4 items"),
        "t18 blocked, D7, D8, T4; D9 is another project's"
    );
    assert!(!r.card_open());
    let (notch, panel) = (
        &r.find("notch").unwrap()["rect"],
        &r.find("terminal").unwrap()["rect"],
    );
    let centre = |v: &Value| v[0].as_f64().unwrap() + v[2].as_f64().unwrap() / 2.0;
    assert!(
        (centre(notch) - centre(panel)).abs() <= 2.0,
        "the notch hangs from the top centre: {notch} in {panel}"
    );
    assert!(
        (notch[1].as_f64().unwrap() - panel[1].as_f64().unwrap()).abs() <= 1.0,
        "at the top of the panel"
    );
    assert!(r.sends().is_empty());
}

#[test]
fn the_notch_is_hidden_with_nothing_open() {
    // ⌘J has nothing to open either.
    let r = run_with(vec![chord(5.0, "j")], false, true);
    assert!(r.find("notch").is_none());
    assert!(!r.card_open());
}

#[test]
fn pressing_the_notch_drops_the_card_with_a_tab_per_item() {
    let r = run(vec![tap(5.0, "notch")]);
    assert!(r.card_open());
    assert_eq!(r.find("qcard").unwrap()["rect"][2], 600.0, "about 600 wide");
    assert!(
        r.find("qcard/tabs").is_some()
            && r.find("segment3").is_some()
            && r.find("segment4").is_none()
    );
    // The blocked agent comes first, with its message and a button for its terminal.
    assert!(r.find("qcard/item/t18").is_some() && r.find("qcard/open/t18").is_some());
    assert!(
        r.find("qcard/keys")
            .is_some_and(|k| k["text"] == "\u{23ce} next  esc close")
    );
}

#[test]
fn the_chord_opens_and_esc_closes() {
    let r = run(vec![chord(5.0, "j"), tap(6.0, TAB_D7)]);
    assert!(r.card_open());
    let first = r.find("qcard/option/D7/0").unwrap();
    assert_eq!(
        first["label"], "Wait a week",
        "the recommended option is shown first"
    );
    assert!(
        r.find("qcard/keys")
            .is_some_and(|k| k["text"] == "1-3 pick  \u{23ce} next  esc close")
    );
    let r = run(vec![chord(5.0, "j"), key(6.0, "Escape")]);
    assert!(!r.card_open());
    assert_eq!(
        r.label().as_deref(),
        Some("4 items"),
        "closing answers nothing"
    );
    assert!(r.sends().is_empty());
}

#[test]
fn picking_an_option_and_sending_answers_the_coordinator() {
    let r = run(vec![
        tap(5.0, "notch"),
        tap(6.0, TAB_D7),
        tap(7.0, "qcard/option/D7/1"),
        tap(7.5, "qcard/send"),
    ]);
    let d7 = r.question("D7");
    assert_eq!(
        d7["answer"], "Ship now",
        "option 1 as shown is the second one in the file"
    );
    assert_eq!(d7["answered_by"], "owner");
    assert!(d7["answered"].as_str().is_some_and(|t| t.ends_with('Z')));
    assert_eq!(
        d7["future_field"], "kept",
        "fields this app does not know survive"
    );
    assert_eq!(r.owner["next_question"], 10);
    assert!(r.question("D8")["answer"].is_null() && r.question("D9")["answer"].is_null());
    assert_eq!(
        r.sends(),
        vec![&json!({"terminal": "t3", "text": "Owner answered D7: Ship now\n"})]
    );
    assert_eq!(r.label().as_deref(), Some("3 items"), "the count drops");
    assert!(r.find("qcard/error").is_none());
}

#[test]
fn the_keys_pick_and_send() {
    // "2" picks the second option shown, Enter sends it.
    let r = run(vec![
        tap(5.0, "notch"),
        tap(6.0, TAB_D7),
        typed(7.0, "2"),
        key(7.5, "Enter"),
    ]);
    assert_eq!(r.question("D7")["answer"], "Ship now");
    assert_eq!(r.sends().len(), 1);
}

#[test]
fn a_typed_answer_is_sent_instead() {
    let r = run(vec![
        tap(5.0, "notch"),
        tap(6.0, TAB_D8),
        tap(7.0, "qcard/input"),
        typed(7.5, "Soft grey"),
        tap(8.5, "qcard/send"),
    ]);
    assert_eq!(r.question("D8")["answer"], "Soft grey");
    assert_eq!(
        r.sends(),
        vec![&json!({"terminal": "t3", "text": "Owner answered D8: Soft grey\n"})]
    );
    assert_eq!(r.label().as_deref(), Some("3 items"));
}

#[test]
fn done_marks_a_todo_and_tells_the_coordinator() {
    let r = run(vec![
        tap(5.0, "notch"),
        tap(6.0, TAB_T4),
        tap(7.0, "qcard/done/T4"),
    ]);
    assert_ne!(r.todo("T4")["done"], "0001-01-01T00:00:00Z");
    assert!(
        r.todo("T4")["done"]
            .as_str()
            .is_some_and(|t| t.ends_with('Z'))
    );
    assert_eq!(
        r.sends(),
        vec![&json!({"terminal": "t3", "text": "Owner did T4: Renew the certificate\n"})]
    );
    assert_eq!(r.label().as_deref(), Some("3 items"));
}

#[test]
fn a_failed_send_keeps_the_answer_and_shows_the_error() {
    // The fixture's `terminal.send` answers `blocked`.
    let r = run_with(
        vec![
            tap(5.0, "notch"),
            tap(6.0, TAB_D7),
            tap(7.0, "qcard/option/D7/1"),
            tap(7.5, "qcard/send"),
        ],
        true,
        false,
    );
    assert_eq!(
        r.question("D7")["answer"],
        "Ship now",
        "the answer stays saved"
    );
    assert_eq!(r.label().as_deref(), Some("3 items"));
    assert!(r.card_open(), "the card stays open to show it");
    let error = r.text("qcard/error");
    assert_eq!(
        error,
        "answer saved; coordinator not reached: agent is blocked: answer its prompt first"
    );
}

#[test]
fn the_blocked_agents_button_shows_its_terminal() {
    let r = run(vec![tap(5.0, "notch"), tap(6.0, "qcard/open/t18")]);
    assert!(!r.card_open(), "the card closes");
    assert_eq!(r.find("agent/t18").unwrap()["itemSelected"], true);
    assert!(
        r.log
            .iter()
            .any(|l| l["method"] == "terminal.attach" && l["params"]["terminal"] == "t18")
    );
    assert_eq!(r.label().as_deref(), Some("4 items"), "it is still blocked");
}
