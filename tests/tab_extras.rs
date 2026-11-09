//! Tab extras (T3) against the in-process fake core: a middle click closes a plain tab (kill, rm)
//! and leaves an agent's tab alone; a secondary press opens `tab/<i>/menu` with Close (off on an
//! agent's tab) and Close others (plain tabs: kill and rm; agent tabs: only detach). The kit names the
//! menu parts `tab/<i>/menu-menu`, `tab/<i>/menu-item0` (Close) and `-item1` (Close others). The
//! restored plain terminal t20 is tab `t-1`; tapping `agent/t3` adds the orchestrator as `t-2`.

mod common;

use common::*;
use serde_json::{Value, json};

fn run(events: Vec<Value>) -> Run {
    App::new()
        .project("contr")
        .fixture_with(|d| {
            let mut fx: Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
            for t in fx["terminals"].as_array_mut().unwrap() {
                t["cwd"] = json!(d.proj);
            }
            fx["screens"] = json!({"t3": "orchestrator-screen\r\n", "t20": "plain-screen\r\n"});
            fx
        })
        .events(events)
        .end(2.0)
        .run()
}

fn middle(t: f64, key: &str) -> Value {
    json!({"t": t, "type": "click", "key": key, "button": "middle"})
}

fn secondary(t: f64, key: &str) -> Value {
    json!({"t": t, "type": "secondary-click", "key": key})
}

#[test]
fn a_middle_click_closes_a_plain_tab_by_killing_and_removing_it() {
    let r = run(vec![middle(0.5, "t-1")]);
    assert!(r.tabs().is_empty(), "{:?}", r.tabs());
    assert_eq!(r.calls("terminal.kill").len(), 1, "{:?}", r.log);
    assert_eq!(r.calls("terminal.rm").len(), 1, "{:?}", r.log);
}

#[test]
fn a_middle_click_leaves_an_agent_tab_alone() {
    let r = run(vec![tap(0.5, "agent/t3"), middle(1.0, "t-2")]);
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "{:?}", r.log);
    assert_eq!(r.closes("t3"), 0, "{:?}", r.log);
}

#[test]
fn the_menu_of_a_plain_tab_has_close_and_close_others_both_on() {
    let r = run(vec![secondary(0.5, "t-1")]);
    for part in ["tab/0/menu-menu", "tab/0/menu-item0", "tab/0/menu-item1"] {
        assert!(r.has(part), "{part}");
    }
    let tab = r.rect("t-1");
    let menu = r.rect("tab/0/menu-menu");
    assert!(menu[1] >= tab[1] && menu[2] > 60.0 && menu[3] > 40.0, "tab {tab:?} menu {menu:?}");
    assert_eq!(r.find("tab/0/menu-item0").unwrap()["focusable"], true);
    assert_eq!(r.find("tab/0/menu-item1").unwrap()["focusable"], true);
}

#[test]
fn close_in_the_menu_closes_a_plain_tab_and_is_off_on_an_agent_tab() {
    let r = run(vec![secondary(0.5, "t-1"), tap(1.0, "tab/0/menu-item0")]);
    assert!(r.tabs().is_empty());
    assert_eq!((r.calls("terminal.kill").len(), r.calls("terminal.rm").len()), (1, 1), "{:?}", r.log);
    assert!(!r.has("tab/0/menu-menu"), "the menu closes");

    let r = run(vec![tap(0.5, "agent/t3"), secondary(1.0, "t-2"), tap(1.4, "tab/1/menu-item0")]);
    assert_eq!(r.find("tab/1/menu-item0").unwrap()["focusable"], false, "Close is off on an agent tab");
    assert_eq!(r.tabs(), vec!["t-1", "t-2"], "{:?}", r.log);
    assert_eq!(r.closes("t3"), 0);
}

#[test]
fn close_others_leaves_one_tab_kills_plain_ones_and_only_detaches_agents() {
    let r = run(vec![tap(0.4, "agent/t3"), tap(0.7, "new-tab"), secondary(1.0, "t-1"), tap(1.3, "tab/0/menu-item1")]);
    assert_eq!(r.tabs(), vec!["t-1"], "{:?}", r.log);
    assert_eq!(r.active_tab(), "t-1");
    assert_eq!(r.closes("t3"), 1, "the agent only detaches: {:?}", r.log);
    let killed: Vec<&Value> = r.calls("terminal.kill");
    assert_eq!(killed.len(), 1, "the new plain tab is killed, not t20 or t3: {:?}", r.log);
    assert!(killed[0]["params"]["terminal"] != "t3" && killed[0]["params"]["terminal"] != "t20", "{}", killed[0]);
    assert_eq!(r.calls("terminal.rm").len(), 1);
}
