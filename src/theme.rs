//! contr's dark tokens (docs/DESIGN.md, "Tokens"), as Rune colours.

use rune::Color;
use rune::tok::TextStyle;

/// `0xRRGGBB` in sRGB → Rune's linear colour.
pub fn srgb(c: u32) -> Color {
    srgb8(((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8)
}

pub fn srgb8(r: u8, g: u8, b: u8) -> Color {
    let d = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    Color { r: d(r), g: d(g), b: d(b), a: 1.0 }
}

macro_rules! token {
    ($($name:ident = $hex:expr;)*) => {
        $(#[allow(non_snake_case)] pub fn $name() -> Color { srgb($hex) })*
    };
}

token! {
    BG_WINDOW = 0x141414;
    BG_BARS = 0x101010;
    TERM_BG = 0x121212;
    TERM_FG = 0xEDEDED;
    BG_CARD = 0x1C1C1C;
    CARD_EDGE = 0x2C2C2C;
    LINE = 0x222222;
    LINE_STRONG = 0x3A3A3A;
    TEXT = 0xEDEDED;
    TEXT_2 = 0xBDBDBD;
    TEXT_DIM = 0x7A7A7A;
    TEXT_FAINT = 0x4A4A4A;
    FAILED = 0xF07178;
    ATTENTION = 0xE5C07B;
}

/// JetBrains Mono 12/18 for the UI, 13/20 for the terminal (falls back to the system mono).
pub const UI: TextStyle = TextStyle::new(12.0, 400.0).mono().lh(18.0);
pub const UI_SMALL: TextStyle = TextStyle::new(11.0, 400.0).mono().lh(17.0);
pub const UI_BOLD: TextStyle = TextStyle::new(12.0, 600.0).mono().lh(18.0);
pub const TERM: TextStyle = TextStyle::new(13.0, 400.0).mono().lh(LINE_H);

pub const LINE_H: f32 = 20.0;
/// A monospace cell is 0.6 em wide in JetBrains Mono and DejaVu Sans Mono.
pub const CELL_W: f32 = 13.0 * 0.6;
