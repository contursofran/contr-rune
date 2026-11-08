//! The settings dialog (K7a): a modal card near the top-centre over a dim scrim. Head with
//! "Settings" and an `esc` chip, the Appearance row (System, Dark, Light), the versions, and a
//! Done button. Keys: `settings-scrim`, `settings-dialog`, `settings-esc`, `appearance`,
//! `settings-done`, `version/contr-rune`, `version/hc`.
//!
//! The design's Transparency sliders and Updates block are left out: the window is opaque and
//! there is no updater.

use rune::{El, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::App;
use crate::settings::Appearance;

/// Card width.
pub const WIDTH: f32 = 520.0;
/// Card corner radius (the design's 12).
const RADIUS: f32 = crate::theme::RADIUS_12;
/// Gap between the top of the window and the card.
pub const TOP: f32 = 88.0;

/// Push the scrim and the card onto the overlay `layer`.
pub fn build(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let close = |s: &mut App| s.settings_open = false;
    let c = &k.c;

    let esc = Button::new("esc").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).on_press(close).build(k).key("settings-esc");
    let head = row()
        .w_full()
        .px(20.0)
        .pt(14.0)
        .pb(10.0)
        .center_y()
        .child(text("Settings").font(k.ty.ui_medium.lh(20.0)).color(c.text_strong))
        .child(spacer())
        .child(esc);

    let selected = Appearance::ALL.iter().position(|a| *a == s.appearance);
    let labels = Appearance::ALL.map(Appearance::label);
    let seg = SegmentedControl::new(&labels, selected)
        .label("Appearance")
        .width(Some(232.0))
        .on_pick(|s: &mut App, i, _tx| s.set_appearance(Appearance::ALL[i]))
        .build(k)
        .key("appearance");
    let appearance = row()
        .w_full()
        .px(20.0)
        .pt(10.0)
        .pb(12.0)
        .center_y()
        .child(text("Appearance").font(k.ty.ui).color(c.text_base))
        .child(spacer())
        .child(seg);

    let version = |key: &str, name: &str, value: &str, faint: bool| {
        row()
            .key(key.to_owned())
            .w_full()
            .h(28.0)
            .center_y()
            .child(text(name.to_owned()).font(k.ty.ui).color(c.text_base))
            .child(spacer())
            .child(text(value.to_owned()).key(format!("{key}/value")).font(k.ty.mono).color(if faint { c.text_faint } else { c.text_muted }))
    };
    let versions = col()
        .w_full()
        .px(20.0)
        .pt(10.0)
        .pb(12.0)
        .gap(2.0)
        .child(text("Versions").font(k.ty.small_medium).color(c.text_faint).pb(4.0))
        .child(version("version/contr-rune", "contr-rune", env!("CARGO_PKG_VERSION"), false))
        .child(version("version/hc", "hc", "not installed", true));

    let done = Button::new("Done").variant(ButtonVariant::Contrast).on_press(close).build(k).key("settings-done");
    let footer = row().w_full().px(20.0).pt(10.0).pb(14.0).child(spacer()).child(done);

    let hairline = || el().w_full().h(1.0).fill(c.border_muted);
    let mut card = col()
        .key("settings-dialog")
        .role("dialog")
        .label("Settings")
        .w(WIDTH)
        .radius(RADIUS)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .clip_rounded(RADIUS)
        .token_motion(k.motion.overlay)
        .child(head)
        .child(hairline())
        .child(appearance)
        .child(hairline())
        .child(versions)
        .child(hairline())
        .child(footer)
        .modal(true, close);
    for shadow in k.shadow_layers(&k.elevation.overlay) {
        card = card.box_shadow(shadow);
    }

    layer.push(el().key("settings-scrim").absolute(0.0, 0.0).w_full().h_full().fill(c.scrim).token_motion(k.motion.overlay).a11y_hidden());
    layer.push(col().key("settings-layer").absolute(0.0, 0.0).w_full().h_full().pt(TOP).items_center().child(card));
}
