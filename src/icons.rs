//! Tab-strip icons: SVG bodies from `docs/design/main.html`, drawn by Rune's icon renderer in the
//! element's colour (`.color()`).

use rune::Icon;

/// The terminal prompt, 16 viewBox, stroke 1.4.
pub static PROMPT: Icon = Icon {
    name: "prompt",
    view: 16.0,
    body: r#"<path d="M3 4.5l3 3.5-3 3.5M8 12h5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

/// The tab's close cross, 10 viewBox, stroke 1.3.
pub static CLOSE: Icon = Icon {
    name: "close",
    view: 10.0,
    body: r#"<path d="M2 2l6 6M8 2l-6 6" fill="none" stroke="currentColor" stroke-width="1.3"/>"#,
};

/// The new-tab plus, 16 viewBox, stroke 1.4.
pub static PLUS: Icon = Icon {
    name: "plus",
    view: 16.0,
    body: r#"<path d="M8 3v10M3 8h10" fill="none" stroke="currentColor" stroke-width="1.4"/>"#,
};
