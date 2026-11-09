//! The "this Mac" row meta (A5). A headless run shows `CONTR_HOST_FAKE` and never live numbers.

mod common;
use common::*;

#[test]
fn this_mac_row_shows_cpu_and_memory() {
    let t = App::new().scene("three-tabs").env("CONTR_HOST_FAKE", "12,48").end(1.0).run();
    assert!(t.texts().iter().any(|x| x == "local · 12% CPU · 48% MEM"), "{:?}", t.texts());
    // No reading (headless never samples): just "local".
    let none = App::new().scene("three-tabs").end(1.0).run();
    assert!(none.texts().iter().any(|x| x == "local"), "{:?}", none.texts());
}
