//! contr's design tokens (`docs/design/tokens.css`), as Rune colours, sizes and type styles.
//!
//! Colours live in [`Theme`]: one field per CSS variable (`--ink-62` is `ink_62`). Only the dark
//! set exists ([`Theme::dark`]); a light theme is a second constructor with the same names, and
//! [`theme`] picks which one the views read. Views use these names, never raw colours.

#![allow(dead_code)] // the whole token set is here; parts 2-9 use the rest

use std::sync::OnceLock;

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

/// CSS `rgba(r, g, b, a)`.
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    srgb8(r, g, b).alpha(a)
}

/// `rgba(255,255,255,a)`: the dark theme's ink, fills and lines.
fn white(a: f32) -> Color {
    Color::WHITE.alpha(a)
}

/// Every colour token. Field names are the CSS variable names with `-` as `_`.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    // Ground and glows.
    pub ground: Color,
    pub glow_slate: Color,
    pub glow_umber: Color,
    pub glow_graphite: Color,
    // Surfaces.
    pub glass_window: Color,
    pub glass_float: Color,
    pub panel: Color,
    /// `panel` composited over `glass-window` over `ground`: the opaque colour the terminal
    /// grid mixes its ANSI colours with. Not a design token; derived.
    pub panel_solid: Color,
    /// `glass-window` composited over `ground`: the window's fill while Rune can't show the
    /// desktop through it. Not a design token; derived.
    pub window_solid: Color,
    // Lines.
    pub panel_line: Color,
    pub window_line: Color,
    pub float_line: Color,
    pub hairline: Color,
    pub rail: Color,
    // Interaction fills.
    pub hover: Color,
    pub hover_strong: Color,
    pub press: Color,
    pub selected: Color,
    pub selected_line: Color,
    pub input_line: Color,
    pub meter_track: Color,
    pub meter_fill: Color,
    pub tick: Color,
    // Ink scale.
    pub ink_100: Color,
    pub ink_95: Color,
    pub ink_90: Color,
    pub ink_85: Color,
    pub ink_62: Color,
    pub ink_60: Color,
    pub ink_50: Color,
    pub ink_45: Color,
    pub ink_42: Color,
    pub ink_40: Color,
    pub ink_38: Color,
    pub ink_35: Color,
    pub ink_32: Color,
    pub ink_30: Color,
    pub ink_26: Color,
    pub ink_24: Color,
    pub ink_22: Color,
    pub terminal_text: Color,
    // Accents.
    pub blocked: Color,
    pub claude: Color,
    pub focus_ring: Color,
    pub lights: Color,
}

impl Theme {
    pub fn dark() -> Theme {
        let ground = srgb(0x0a0b0d);
        let over = |top: Color, bottom: Color| {
            // Composite in sRGB, as CSS does: Rune's colours are linear, so go through 8-bit sRGB.
            let to = |c: f32| if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            let from = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
            let mix = |t: f32, b: f32| from(to(t) * top.a + to(b) * (1.0 - top.a));
            Color { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a: 1.0 }
        };
        let glass_window = rgba(20, 21, 24, 0.7);
        let panel = rgba(20, 21, 24, 0.85);
        let window_solid = over(glass_window, ground);
        Theme {
            ground,
            glow_slate: srgb(0x3a5a8c),
            glow_umber: srgb(0x6b4a2a),
            glow_graphite: srgb(0x2c3240),
            glass_window,
            glass_float: rgba(34, 35, 40, 0.72),
            panel,
            panel_solid: over(panel, window_solid),
            window_solid,
            panel_line: white(0.065),
            window_line: white(0.09),
            float_line: white(0.075),
            hairline: white(0.06),
            rail: white(0.07),
            hover: white(0.045),
            hover_strong: white(0.06),
            press: white(0.08),
            selected: white(0.085),
            selected_line: white(0.06),
            input_line: white(0.1),
            meter_track: white(0.07),
            meter_fill: white(0.55),
            tick: white(0.14),
            ink_100: srgb(0xf4f4f5),
            ink_95: srgb(0xe9e9ea),
            ink_90: white(0.9),
            ink_85: white(0.85),
            ink_62: white(0.62),
            ink_60: white(0.6),
            ink_50: white(0.5),
            ink_45: white(0.45),
            ink_42: white(0.42),
            ink_40: white(0.4),
            ink_38: white(0.38),
            ink_35: white(0.35),
            ink_32: white(0.32),
            ink_30: white(0.3),
            ink_26: white(0.26),
            ink_24: white(0.24),
            ink_22: white(0.22),
            terminal_text: white(0.82),
            blocked: srgb(0xf07178),
            claude: srgb(0xd97757),
            focus_ring: white(0.5),
            lights: white(0.14),
        }
    }
}

/// The theme the views read. Dark only for now.
pub fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::dark)
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
pub const TAB_HEIGHT: f32 = 30.0;
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

    #[test]
    fn alpha_is_kept() {
        let t = Theme::dark();
        assert!(near(t.panel.a, 0.85));
        assert!(near(t.panel_line.a, 0.065));
        assert!(near(t.ink_62.a, 0.62));
        assert_eq!(t.ink_100.a, 1.0);
    }

    #[test]
    fn window_solid_is_glass_over_ground() {
        // rgba(20,21,24,.7) over #0a0b0d, composited in sRGB: about (17, 18, 21).
        let w = Theme::dark().window_solid;
        let back = |c: f32| {
            let v = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            (v * 255.0).round() as i32
        };
        assert_eq!([back(w.r), back(w.g), back(w.b)], [17, 18, 21]);
        assert_eq!(w.a, 1.0);
    }
}
