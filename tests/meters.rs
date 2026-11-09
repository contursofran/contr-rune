//! The plan meters in the sidebar foot (H11): `meter/claude` and `meter/codex` read fixture usage
//! files named by `CONTR_STATE_DIR` and `CODEX_HOME`, never the owner's files.

mod common;
use common::*;

fn foot(state: &std::path::Path, codex: &std::path::Path) -> Run {
    App::new().scene("three-tabs").env("CONTR_STATE_DIR", state.to_string_lossy()).env("CODEX_HOME", codex.to_string_lossy()).end(1.0).run()
}

#[test]
fn foot_shows_both_plans_left_of_the_gear() {
    let root = std::env::temp_dir().join(format!("contr-h11-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (state, codex) = (root.join("state"), root.join("codex"));
    let day = codex.join("sessions/2026/11/09");
    std::fs::create_dir_all(state.join("usage")).unwrap();
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(state.join("usage/claude.json"), r#"{"schema":1,"updated_at":1,"windows":[{"kind":"five_hour","used_percent":38.0,"resets_at":null}]}"#).unwrap();
    std::fs::write(
        day.join("rollout-x.jsonl"),
        r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":12.0,"resets_at":99999999999}}}}"#,
    )
    .unwrap();

    let t = foot(&state, &codex);
    assert_eq!(t.text_in("meter/claude", "label").as_deref(), Some("Claude"));
    assert_eq!(t.text_in("meter/claude", "value").as_deref(), Some("38%"));
    assert_eq!(t.text_in("meter/codex", "label").as_deref(), Some("Codex"));
    assert_eq!(t.text_in("meter/codex", "value").as_deref(), Some("12%"));
    // One row: Claude, then Codex, then the gear at the right; the bar is 44 wide.
    let (claude, codex_r, gear) = (t.rect("meter/claude"), t.rect("meter/codex"), t.rect("settings"));
    assert_eq!(claude[1], codex_r[1]);
    assert!(claude[0] + claude[2] <= codex_r[0] && codex_r[0] + codex_r[2] <= gear[0], "{claude:?} {codex_r:?} {gear:?}");
    assert_eq!(t.rect_in("meter/claude", "bar")[2], 44.0);

    // No files: both show "–".
    let none = foot(&root.join("none"), &root.join("none"));
    assert_eq!(none.text_in("meter/claude", "value").as_deref(), Some("–"));
    assert_eq!(none.text_in("meter/codex", "value").as_deref(), Some("–"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn context_meter_under_the_tree_follows_the_selected_agent() {
    let root = std::env::temp_dir().join(format!("contr-h13-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("usage/context")).unwrap();
    std::fs::write(root.join("usage/context/t3.json"), r#"{"schema":1,"updated_at":1,"tokens":90000,"size":200000,"model":"opus","effort":"high"}"#).unwrap();
    let run = |events| {
        App::new()
            .project("contr")
            .env("CONTR_STATE_DIR", root.to_string_lossy())
            .fixture_with(|d| {
                let mut fx: serde_json::Value = serde_json::from_str(include_str!("fixtures/hc/agents.json")).unwrap();
                for t in fx["terminals"].as_array_mut().unwrap() {
                    t["cwd"] = serde_json::json!(d.proj);
                }
                fx
            })
            .events(events)
            .end(1.5)
            .run()
    };
    let r = run(vec![tap(0.5, "agent/t3")]);
    assert_eq!(r.text_in("meter/context", "label").as_deref(), Some("Context"));
    assert_eq!(r.text_in("meter/context", "value").as_deref(), Some("90k of 200k  45%"));
    assert!(r.rect_in("meter/context", "bar")[1] > r.rect("meter/context")[1], "the bar is under the label");
    // No agent selected: no row.
    assert!(!run(vec![]).has("meter/context"));
    // An agent without a context file (t17): no row.
    assert!(!run(vec![tap(0.5, "agent/t17")]).has("meter/context"));
    let _ = std::fs::remove_dir_all(&root);
}
