//! The window shell shared by the app and `examples/kit_preview.rs`: a `WindowTitlebar` (38) and
//! three panel cards, all coloured from the [`Kit`].

use rune::{ChromeInsets, El, col, el, row};
use rune_kit::shell::WindowTitlebar;
use rune_kit::{Kit, root};

use crate::theme::{FILES_WIDTH, SIDEBAR_WIDTH, SPACE_8, TITLEBAR_HEIGHT};

/// Width of the title bar's leading area (traffic lights); the tabs start after it.
pub const LIGHTS_WIDTH: f32 = 316.0;

/// A flat panel card: `bg_base` fill, `border_muted` border, radius `lg`.
pub fn panel(k: &Kit) -> El {
    col().fill(k.c.bg_base).radius(k.radius.lg).border(1.0, k.c.border_muted).clip_rounded(k.radius.lg)
}

/// The sidebar card (fixed width).
pub fn sidebar_panel(k: &Kit) -> El {
    panel(k).key("sidebar").w(SIDEBAR_WIDTH).h_full().min_h(0.0)
}

/// The terminal card (takes the rest).
pub fn terminal_panel(k: &Kit) -> El {
    panel(k).key("terminal").grow(1.0).min_w(0.0).h_full().min_h(0.0)
}

/// The files card (fixed width).
pub fn files_panel(k: &Kit) -> El {
    panel(k).key("files").w(FILES_WIDTH).h_full().min_h(0.0)
}

/// The title bar: 38 high, `leading` (traffic lights or a stand-in for them), the tab strip,
/// then `trailing` items.
pub fn titlebar(k: &Kit, leading: El, tabs: El, trailing: Vec<El>) -> El {
    let mut bar = WindowTitlebar::from_insets(ChromeInsets::default(), 1.0)
        .height(TITLEBAR_HEIGHT)
        .leading_padding(18.0)
        .trailing_padding(8.0)
        .gap(8.0)
        .leading(leading)
        .tabs(tabs);
    for t in trailing {
        bar = bar.trailing(t);
    }
    bar.build(k)
}

#[allow(dead_code)] // the example draws its own lights
/// A blank leading area where the native traffic lights sit.
pub fn lights_gap() -> El {
    el().w(LIGHTS_WIDTH).h(12.0)
}

/// The window: the title bar over a row of the three cards.
pub fn window(k: &Kit, bar: El, sidebar: El, terminal: El, files: El, layer: Vec<El>) -> El {
    let body = row().w_full().grow(1.0).min_h(0.0).gap(SPACE_8).px(SPACE_8).pb(SPACE_8).child(sidebar).child(terminal).child(files);
    root(col().key("window").w_full().h_full().fill(k.c.bg_deep).child(bar).child(body), layer)
}
