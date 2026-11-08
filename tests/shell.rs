//! The window shell must keep the bounds and colours of `examples/kit_preview.rs` (neutral kit,
//! board 1N at 1710x1112): a 38 high title bar and three panel cards. The numbers below are
//! what `kit_preview --tree` reports for the same size.

use std::process::Command;

use rune_kit::Kit;
use serde_json::Value;

fn tree(name: &str) -> Value {
    let out = std::env::temp_dir().join(format!("contr-rune-{name}-{}.json", std::process::id()));
    let run = Command::new(env!("CARGO_BIN_EXE_contr-rune"))
        .env("CONTR_SETTINGS", "/nonexistent/contr-rune-settings.json")
        .args(["--tree", out.to_str().unwrap(), "--size", "1710x1112", "--scale", "1", "--scene", "three-tabs", "--at", "1"])
        .output()
        .expect("run the app");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let v = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_file(out);
    v
}

fn element<'a>(tree: &'a Value, key: &str) -> &'a Value {
    tree["elements"].as_array().unwrap().iter().find(|e| e["key"] == key).unwrap_or_else(|| panic!("no element {key}"))
}

fn rect(e: &Value) -> [f64; 4] {
    let r = e["rect"].as_array().unwrap();
    [0, 1, 2, 3].map(|i| r[i].as_f64().unwrap())
}

#[test]
fn shell_matches_kit_preview() {
    let t = tree("shell");
    let k = Kit::neutral(true);

    let bar = element(&t, "titlebar");
    assert_eq!(rect(bar), [0.0, 0.0, 1710.0, 38.0]);
    assert_eq!(bar["fill"], k.c.bg_deep.hex().as_str());
    assert_eq!(element(&t, "window")["fill"], k.c.bg_deep.hex().as_str());
    // The tabs start after the traffic-light area.
    assert_eq!(rect(element(&t, "titlebar-tabs"))[0], 342.0);

    let panels = [
        ("sidebar", [8.0, 38.0, 334.0, 1066.0]),
        ("terminal", [350.0, 38.0, 1042.0, 1066.0]),
        ("files", [1400.0, 38.0, 302.0, 1066.0]),
    ];
    for (key, bounds) in panels {
        let e = element(&t, key);
        assert_eq!(rect(e), bounds, "{key} bounds");
        assert_eq!(e["fill"], k.c.bg_base.hex().as_str(), "{key} uses the kit's panel colour");
    }
}
