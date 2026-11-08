//! Part 2 (tabs), headless: runs the app on Rune's virtual clock with `--script` and `--tree`, and
//! checks the tab strip's bounds against `docs/design/main.html` and its click/hover/key behaviour.
//!
//! The scene `three-tabs` opens three real shells (PTYs); the tree only reads tab bounds and
//! counts, which don't depend on what the shells print.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Runs the scene with `script` (JSON events), returns the `--tree` JSON at the end of the script.
fn run(name: &str, script: &str) -> Vec<Value> {
    let dir: PathBuf = std::env::temp_dir().join(format!("contr-tabs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (script_path, tree_path) = (dir.join("script.json"), dir.join("tree.json"));
    std::fs::write(&script_path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .args(["--size", "1710x1112", "--scale", "1", "--scene", "three-tabs", "--script"])
        .arg(&script_path)
        .arg("--tree")
        .arg(&tree_path)
        .env("CONTR_PROJECTS", std::env::temp_dir())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tree: Value = serde_json::from_str(&std::fs::read_to_string(&tree_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    tree["elements"].as_array().unwrap().clone()
}

fn find<'a>(els: &'a [Value], key: &str) -> Option<&'a Value> {
    els.iter().find(|e| e["key"] == key)
}

fn rect(els: &[Value], key: &str) -> [f64; 4] {
    let r = find(els, key).unwrap_or_else(|| panic!("no element keyed {key}"))["rect"].as_array().unwrap().clone();
    [r[0].as_f64().unwrap(), r[1].as_f64().unwrap(), r[2].as_f64().unwrap(), r[3].as_f64().unwrap()]
}

fn tab_count(els: &[Value]) -> usize {
    els.iter().filter(|e| e["key"].as_str().is_some_and(|k| k.starts_with("tab-") && k[4..].parse::<usize>().is_ok())).count()
}

fn selected_tab(els: &[Value]) -> usize {
    // The selected tab has the `panel` fill.
    (0..tab_count(els)).find(|i| find(els, &format!("tab-{i}")).unwrap()["fill"] != "#00000000").unwrap()
}

#[test]
fn bounds_match_the_design() {
    let els = run("bounds", "[{\"t\": 0.5, \"type\": \"mark\", \"name\": \"idle\"}]");
    assert_eq!(rect(&els, "titlebar"), [0.0, 0.0, 1710.0, 38.0]);
    assert_eq!(tab_count(&els), 3);
    let (t0, t1, t2) = (rect(&els, "tab-0"), rect(&els, "tab-1"), rect(&els, "tab-2"));
    assert_eq!(t0[0], 350.0, "strip starts at x=350");
    for t in [t0, t1, t2] {
        assert_eq!(t[3], 30.0, "tab height");
        assert!(t[2] <= 180.0, "tab max width");
        assert_eq!(t[1], 4.0, "tabs centred in the 38 px bar");
    }
    assert_eq!(t1[0] - (t0[0] + t0[2]), 2.0, "tab gap");
    assert_eq!(t2[0] - (t1[0] + t1[2]), 2.0, "tab gap");
    // Only the selected tab (the last opened) shows its close box: 16 px.
    let x = rect(&els, "tab-2-close");
    assert_eq!([x[2], x[3]], [16.0, 16.0]);
    assert!(find(&els, "tab-0-close").is_none() && find(&els, "tab-1-close").is_none());
    // `+`: 26 px, 2 px gap plus 2 px margin after the last tab, centred.
    let p = rect(&els, "tab-new");
    assert_eq!([p[2], p[3]], [26.0, 26.0]);
    assert_eq!(p[0] - (t2[0] + t2[2]), 4.0);
    assert_eq!(p[1], 6.0);
    // The strip ends before the right side of the title bar.
    assert!(p[0] + p[2] < 1710.0);
}

#[test]
fn click_selects_a_tab() {
    let els = run("select", r#"[{"t": 0.5, "type": "click", "key": "tab-0"}]"#);
    assert_eq!(tab_count(&els), 3);
    assert_eq!(selected_tab(&els), 0);
}

#[test]
fn close_button_closes_a_tab() {
    let els = run("close", r#"[{"t": 0.5, "type": "click", "key": "tab-2-close"}]"#);
    assert_eq!(tab_count(&els), 2);
}

#[test]
fn plus_opens_a_tab() {
    let els = run("plus", r#"[{"t": 0.5, "type": "click", "key": "tab-new"}]"#);
    assert_eq!(tab_count(&els), 4);
    assert_eq!(selected_tab(&els), 3);
}

#[test]
fn hover_does_not_move_tabs_or_show_close() {
    let before = run("hover-before", r#"[{"t": 0.5, "type": "mark", "name": "idle"}]"#);
    let els = run("hover", r#"[{"t": 0.5, "type": "move", "key": "tab-0"}]"#);
    assert!(find(&els, "tab-0-close").is_none(), "only the selected tab has a close box");
    assert!(find(&els, "tab-2-close").is_some());
    for k in ["tab-0", "tab-1", "tab-2", "tab-new"] {
        assert_eq!(rect(&els, k), rect(&before, k), "{k} does not move on hover");
    }
}

#[test]
fn selected_tab_has_the_panel_fill_and_line() {
    let els = run("fill", r#"[{"t": 0.5, "type": "mark", "name": "idle"}]"#);
    // `panel` is rgba(20,21,24,.85).
    assert_eq!(find(&els, "tab-2").unwrap()["fill"], "#141518d9");
    assert_eq!(find(&els, "tab-0").unwrap()["fill"], "#00000000", "unselected tab has no fill");
    assert_eq!(find(&els, "tab-1").unwrap()["fill"], "#00000000");
}

/// The tree has no borders, so read the snapshot: the selected tab's top edge row (1 px
/// `panel-line`, white at 6.5%) is brighter than the fill just inside it.
#[test]
fn selected_tab_has_the_inset_line() {
    let dir = std::env::temp_dir().join(format!("contr-tabs-snap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (snap, tree) = (dir.join("snap.png"), dir.join("tree.json"));
    let out = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .args(["--size", "1710x1112", "--scale", "1", "--scene", "three-tabs", "--at", "1", "--snap"])
        .arg(&snap)
        .arg("--tree")
        .arg(&tree)
        .env("CONTR_PROJECTS", std::env::temp_dir())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let t: Value = serde_json::from_str(&std::fs::read_to_string(&tree).unwrap()).unwrap();
    let els = t["elements"].as_array().unwrap().clone();
    let r = rect(&els, "tab-2");
    let decoder = png::Decoder::new(std::fs::File::open(&snap).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let ch = info.color_type.samples();
    // A column in the tab's right padding, clear of text and icons.
    let x = (r[0] + r[2] - 24.0) as usize;
    let lum = |y: usize| {
        let i = (y * info.width as usize + x) * ch;
        buf[i] as u32 + buf[i + 1] as u32 + buf[i + 2] as u32
    };
    let (edge, inside) = (lum(r[1] as usize), lum(r[1] as usize + 3));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(edge > inside + 15, "top edge {edge} should be brighter than the fill {inside}");
}

#[test]
fn shortcuts_open_and_close_tabs() {
    // ⌘T / ⌘W on macOS, Ctrl+Shift elsewhere.
    let shift = !cfg!(target_os = "macos");
    let key = |t: f64, name: &str| format!("{{\"t\": {t}, \"type\": \"key\", \"name\": \"{name}\", \"cmd\": true, \"shift\": {shift}}}");
    let els = run("keys-new", &format!("[{}]", key(0.5, "t")));
    assert_eq!(tab_count(&els), 4);
    let els = run("keys-close", &format!("[{}]", key(0.5, "w")));
    assert_eq!(tab_count(&els), 2);
}

#[test]
fn ctrl_tab_cycles_tabs() {
    let els = run("cycle", r#"[{"t": 0.5, "type": "key", "name": "Tab", "ctrl": true}]"#);
    assert_eq!(selected_tab(&els), 0, "from the last tab, Ctrl+Tab wraps to the first");
    let els = run("cycle-back", r#"[{"t": 0.5, "type": "key", "name": "Tab", "ctrl": true, "shift": true}]"#);
    assert_eq!(selected_tab(&els), 1);
}

#[test]
fn ctrl_page_keys_cycle_tabs() {
    // The scene ends on the last tab: PageDown wraps to the first, PageUp goes back one.
    let els = run("pgdn", r#"[{"t": 0.5, "type": "key", "name": "PageDown", "ctrl": true}]"#);
    assert_eq!(selected_tab(&els), 0);
    let els = run("pgup", r#"[{"t": 0.5, "type": "key", "name": "PageUp", "ctrl": true}]"#);
    assert_eq!(selected_tab(&els), 1);
    let els = run(
        "pgdn-pgup",
        r#"[{"t": 0.5, "type": "key", "name": "PageDown", "ctrl": true},
            {"t": 1.0, "type": "key", "name": "PageDown", "ctrl": true},
            {"t": 1.5, "type": "key", "name": "PageUp", "ctrl": true}]"#,
    );
    assert_eq!(selected_tab(&els), 0, "0 -> 1 -> 0");
}

#[test]
fn many_tabs_shrink_and_stay_in_the_strip() {
    // Forty clicks on `+`, by key each time (the button moves as tabs shrink).
    let clicks: Vec<String> = (0..40).map(|i| format!("{{\"t\": {}, \"type\": \"click\", \"key\": \"tab-new\"}}", 0.5 + i as f64 * 0.3)).collect();
    let els = run("many", &format!("[{}]", clicks.join(",")));
    assert_eq!(tab_count(&els), 43);
    let p = rect(&els, "tab-new");
    assert!(p[0] + p[2] <= 1710.0 - 8.0, "the + button stays inside the window");
    assert!(rect(&els, "tab-0")[2] < 60.0, "tabs shrink");
}
