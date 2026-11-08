//! Sizes, type styles, fonts and the sRGB helper. Colours, type and radius come from the
//! rune-kit `Kit` (`Kit::neutral`), never from here; the terminal ANSI palette is in `grid.rs`.

#![allow(dead_code)] // the whole token set is here; parts 2-9 use the rest

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

// Spacing (`--space-*`), radius (`--radius-*`) and sizes.
pub const SPACE_2: f32 = 2.0;
pub const SPACE_4: f32 = 4.0;
pub const SPACE_6: f32 = 6.0;
pub const SPACE_7: f32 = 7.0;
pub const SPACE_8: f32 = 8.0;
pub const SPACE_9: f32 = 9.0;
pub const SPACE_10: f32 = 10.0;
pub const SPACE_12: f32 = 12.0;
pub const SPACE_14: f32 = 14.0;
pub const SPACE_16: f32 = 16.0;
pub const SPACE_18: f32 = 18.0;
pub const SPACE_20: f32 = 20.0;

pub const RADIUS_1: f32 = 1.0;
pub const RADIUS_2: f32 = 2.0;
pub const RADIUS_4: f32 = 4.0;
pub const RADIUS_6: f32 = 6.0;
pub const RADIUS_7: f32 = 7.0;
pub const RADIUS_8: f32 = 8.0;
pub const RADIUS_9: f32 = 9.0;
pub const RADIUS_12: f32 = 12.0;
pub const RADIUS_14: f32 = 14.0;

pub const TITLEBAR_HEIGHT: f32 = 38.0;
/// Outer widths, as the reference renders them (CSS without `box-sizing`): 312 / 280 content
/// plus padding and border. The sidebar ends at x=342, under the title bar's left area.
pub const SIDEBAR_WIDTH: f32 = 334.0;
pub const FILES_WIDTH: f32 = 302.0;
pub const ROW_HEIGHT: f32 = 30.0;
pub const FILE_ROW_HEIGHT: f32 = 26.0;
pub const ICON_BUTTON: f32 = 28.0;
pub const TAB_MAX_WIDTH: f32 = 180.0;
pub const NOTCH_HEIGHT: f32 = 30.0;
pub const NOTCH_OPEN_WIDTH: f32 = 600.0;
pub const OUTLINE_CARD_WIDTH: f32 = 380.0;
pub const TICK_HEIGHT: f32 = 2.0;
pub const METER_HEIGHT: f32 = 4.0;

/// Width of the title bar's left area, kept free for the macOS traffic lights (the tabs start here).
pub const TITLEBAR_LEFT: f32 = 342.0;

// Type styles (`.question` … `.count`). Inter for text, JetBrains Mono for code and numbers.
pub const QUESTION: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(18.0);
pub const BODY_STRONG: TextStyle = TextStyle::new(13.0, 500.0).lh(18.0);
pub const ROW_SMALL: TextStyle = TextStyle::new(12.5, 400.0).lh(18.0);
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0).lh(17.0);
pub const LABEL: TextStyle = TextStyle::new(12.0, 500.0).lh(18.0);
pub const CHIP: TextStyle = TextStyle::new(11.5, 400.0).lh(16.0);
pub const TERMINAL: TextStyle = TextStyle::new(13.0, 400.0).mono().lh(LINE_H);
pub const GLYPH: TextStyle = TextStyle::new(17.0, 400.0).mono().lh(18.0);
pub const VALUE: TextStyle = TextStyle::new(11.5, 400.0).mono().lh(16.0);
pub const META: TextStyle = TextStyle::new(11.0, 400.0).mono().lh(16.0);
pub const COUNT: TextStyle = TextStyle::new(11.0, 500.0).mono().lh(14.0);

/// Terminal line height (`terminal`: 13/20).
pub const LINE_H: f32 = 20.0;
/// A monospace cell is 0.6 em wide in JetBrains Mono.
pub const CELL_W: f32 = 13.0 * 0.6;

/// Font files bundled in `assets/fonts` (SIL OFL; licences next to them). Passing any replaces
/// Rune's built-in Inter, so Inter is listed too.
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];
pub const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono, JetBrainsMono Nerd Font, ui-monospace");

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool { (a - b).abs() < 1e-4 }

    #[test]
    fn srgb_converts_to_linear() {
        // sRGB 128 is 0.2158605 in linear light; the ends stay put.
        assert!(near(srgb8(128, 128, 128).r, 0.215_860_5));
        assert!(near(srgb8(0, 0, 0).g, 0.0));
        assert!(near(srgb8(255, 255, 255).b, 1.0));
        assert_eq!(srgb8(10, 20, 30).a, 1.0);
    }
}
