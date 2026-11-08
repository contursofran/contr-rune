//! contr's colour tokens (`docs/design/tokens.css`), kept for the `contr` look of
//! `examples/kit_preview.rs`. The app itself reads the rune-kit `Kit` only.

#![allow(dead_code)]

use rune::Color;

use crate::theme::{srgb, srgb8};

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

/// The theme the example reads. Dark only.
pub fn theme() -> &'static Theme {
    static THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    THEME.get_or_init(Theme::dark)
}
