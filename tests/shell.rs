//! The window shell must keep the bounds and colours of `examples/kit_preview.rs` (neutral kit,
//! board 1N at 1710x1112): a 38 high title bar and three panel cards. The numbers below are
//! what `kit_preview --tree` reports for the same size.

use rune_kit::Kit;

mod common;
use common::*;

#[test]
fn shell_matches_kit_preview() {
    let t = App::new().scene("three-tabs").end(1.0).run();
    let k = Kit::neutral(true);
    let fill = |key: &str| t.find(key).unwrap()["fill"].clone();

    assert_eq!(t.rect("titlebar"), [0.0, 0.0, 1710.0, 38.0]);
    assert_eq!(fill("titlebar"), k.c.bg_deep.hex().as_str());
    assert_eq!(fill("window"), k.c.bg_deep.hex().as_str());
    // The tabs start after the traffic-light area.
    assert_eq!(t.rect("titlebar-tabs")[0], 342.0);

    let panels = [
        ("sidebar", [8.0, 38.0, 334.0, 1066.0]),
        ("terminal", [350.0, 38.0, 1042.0, 1066.0]),
        ("files", [1400.0, 38.0, 302.0, 1066.0]),
    ];
    for (key, bounds) in panels {
        assert_eq!(t.rect(key), bounds, "{key} bounds");
        assert_eq!(fill(key), k.c.bg_base.hex().as_str(), "{key} uses the kit's panel colour");
    }
}
