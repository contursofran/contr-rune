//! Exploration: contr's main screen (`docs/design/main.html`) built from rune-kit components,
//! with static fake data. Not the app: nothing here is wired to a PTY or to git.
//!
//! Two looks of the same view tree:
//!
//! ```sh
//! cargo run --example kit_preview -- --snap kit-neutral.png --size 1710x1112 --scale 1 --theme dark neutral
//! cargo run --example kit_preview -- --snap kit-contr.png    --size 1710x1112 --scale 1 --theme dark contr
//! ```
//!
//! `neutral` is `Kit::neutral(true)` untouched. `contr` is the same kit with our tokens
//! (`src/theme.rs`) written into its `Palette`, `Type` and `Radius`.

#[path = "kit_preview_tokens/colors.rs"]
mod colors;
#[path = "../src/shell.rs"]
mod shell;
#[path = "../src/theme.rs"]
mod theme;

use rune::{AppSpec, Color, Cx, El, Icon, TextStyle, col, el, icon, row, spacer, text};
use rune_kit::shell::{TabDragState, TabItem, TabOrientation, TabOverflow, TabStrip};
use rune_kit::{
    Button, ButtonVariant, ControlSize, FileNode, FileTree, IconButton, IconButtonVariant, Kit, Progress, Sidebar, SidebarRow, Tree,
    TreeItem, TreeModel, TreeStyle,
};

use colors::{Theme, theme as tokens};

// Icons from main.html (16 point view box).
macro_rules! ico {
    ($name:ident, $w:literal, $body:literal) => {
        static $name: Icon = Icon {
            name: concat!("kp-", stringify!($name)),
            view: 16.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width=""#,
                $w,
                r#"" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        };
    };
}
ico!(PLUS, "1.4", r#"<path d="M8 3v10M3 8h10"/>"#);
ico!(SEARCH, "1.3", r#"<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5l3.5 3.5"/>"#);
ico!(SPLIT, "1.3", r#"<rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M8 2.5v11"/>"#);
ico!(PANEL_RIGHT, "1.3", r#"<rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M10 2.5v11"/>"#);
ico!(FILTER, "1.3", r#"<path d="M2.5 4h11M4.5 8h7M6.5 12h3"/>"#);
ico!(GEAR, "1.3", r#"<circle cx="8" cy="8" r="2.2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>"#);
ico!(FOLDER, "1.3", r#"<path d="M1.5 4.5a1 1 0 0 1 1-1h3.6l1.5 1.6h5.9a1 1 0 0 1 1 1v6.4a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"/>"#);
ico!(LAPTOP, "1.3", r#"<rect x="3" y="3" width="10" height="7" rx="1.2"/><path d="M1.5 12.5h13"/>"#);
ico!(SERVER, "1.3", r#"<rect x="2" y="2.5" width="12" height="4.5" rx="1.5"/><rect x="2" y="9" width="12" height="4.5" rx="1.5"/><path d="M4.5 4.75h1M4.5 11.25h1"/>"#);
ico!(TERM, "1.4", r#"<path d="M3 4.5l3 3.5-3 3.5M8 12h5"/>"#);
ico!(CLAUDE, "1.7", r#"<path d="M9.19 8.17L14.93 8.97M8.95 8.74L12.10 11.20M8.45 9.11L10.40 13.93M7.83 9.19L7.33 12.75M7.26 8.95L3.81 13.36M6.89 8.45L2.99 10.02M6.81 7.83L1.86 7.14M7.05 7.26L4.06 4.92M7.55 6.89L5.38 1.51M8.17 6.81L8.72 2.85M8.74 7.05L11.69 3.27M9.11 7.55L12.54 6.16"/>"#);

#[derive(Clone, Copy)]
struct Change {
    added: u32,
    removed: u32,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Plain,
    Modified,
    New,
    Deleted,
}

struct App {
    contr: bool,
    files: TreeModel<FileNode<Change>>,
    tabs: TabDragState,
}

fn init() -> App {
    let contr = !rune::extra_args().iter().any(|a| a == "neutral");
    let c = |added, removed, kind| Change { added, removed, kind };
    let paths = [
        ("cmd/main.go", c(0, 0, Kind::Plain)),
        ("docs/design.md", c(0, 0, Kind::Plain)),
        ("internal/agents/agents.go", c(0, 0, Kind::Plain)),
        ("internal/app/board.go", c(0, 0, Kind::Plain)),
        ("internal/app/panel.go", c(38, 12, Kind::Modified)),
        ("internal/app/tabs.go", c(31, 0, Kind::New)),
        ("internal/app/topbar.go", c(0, 52, Kind::Deleted)),
        ("AGENTS.md", c(0, 0, Kind::Plain)),
        ("HANDOFF.md", c(0, 34, Kind::Deleted)),
        ("NOTICE", c(0, 0, Kind::Plain)),
    ];
    let mut files = FileTree::new().compact_folders(false).build(paths);
    files.set_expanded("internal", true);
    files.set_expanded("internal/app", true);
    App { contr, files, tabs: TabDragState::new() }
}

/// contr's tokens written into a kit. The neutral kit is the base, so every slot contr has no
/// token for keeps rune-kit's value.
fn contr_kit() -> Kit {
    let t: &Theme = tokens();
    let mut k = Kit::neutral(true);
    let c = &mut k.c;
    c.page = t.ground;
    c.bg_deep = t.window_solid;
    c.bg_base = t.panel_solid;
    c.layer_01 = t.panel_solid;
    c.layer_02 = t.meter_track;
    c.layer_03 = t.hover_strong;
    c.surface_base = t.meter_track;
    c.surface_weak = t.meter_track;
    c.border_weak = t.panel_line;
    c.border_active = t.meter_fill;
    c.border_muted = t.panel_line;
    c.border_base = t.panel_line;
    c.border_strong = t.input_line;
    c.border_focus = t.focus_ring;
    c.hover = t.hover;
    c.pressed = t.selected;
    c.text_base = t.ink_95;
    c.text_strong = t.ink_95;
    c.text_muted = t.ink_60;
    c.text_weak = t.ink_62;
    c.std_text = t.ink_62;
    c.text_faint = t.ink_42;
    c.icon_base = t.ink_90;
    c.icon_muted = t.ink_50;
    c.icon_faint = t.ink_38;
    c.text_accent = t.ink_100;
    c.icon_accent = t.ink_100;
    c.bg_accent = t.claude;
    // Red only means "needs you"; no amber or green in the chrome.
    c.warning.icon = t.blocked;
    c.success.icon = t.ink_90;
    // The tree draws removed counts in `danger`; contr keeps them dim.
    c.danger.icon = t.ink_32;
    k.ty.ui = theme::ROW_SMALL;
    k.ty.ui_medium = TextStyle { weight: 500.0, ..theme::ROW_SMALL };
    k.ty.ui_compact = theme::ROW_SMALL;
    k.ty.small = theme::SMALL;
    k.ty.small_medium = theme::LABEL;
    k.ty.text_xsmall = theme::SMALL;
    k.ty.text_xsmall_medium = theme::LABEL;
    k.ty.mono = theme::META;
    k.radius = rune_kit::Radius { xs: 4.0, sm: 6.0, md: 8.0, lg: 12.0, xl: 14.0 };
    k
}

fn glyph(s: &'static str, size: f32, color: Color) -> El {
    text(s).font(TextStyle::new(size, 400.0).mono().lh(18.0)).color(color)
}

fn claude(k: &Kit, size: f32, dim: bool) -> El {
    let t = tokens();
    icon(&CLAUDE, size).color(if k.dark && dim { t.claude.alpha(0.45) } else { t.claude })
}

fn lights() -> El {
    let dot = |c: u32| el().size(12.0, 12.0).radius(6.0).fill(theme::srgb(c));
    row().w(shell::LIGHTS_WIDTH).gap(8.0).center_y().child(dot(0xff5f57)).child(dot(0xfebc2e)).child(dot(0x28c840))
}

fn titlebar(s: &App, cx: &mut Cx<App>, k: &Kit, layer: &mut Vec<El>) -> El {
    let t = tokens();
    let tabs = vec![
        TabItem::new("agent", "Agent board polish").avatar(claude(k, 14.0, false)),
        TabItem::new("zsh", "zsh").icon(&TERM),
        TabItem::new("test", "go test ./...").avatar(glyph("⠋", 14.0, t.ink_100)),
    ];
    let plus = IconButton::new(&PLUS).variant(IconButtonVariant::GhostMuted).control_size(ControlSize::Small).label("New tab").build(k);
    let strip = TabStrip::new("top", TabOrientation::Horizontal)
        .tabs(tabs)
        .active(Some("agent"))
        .overflow(TabOverflow::ShrinkThenScroll { min_width: 28.0, max_width: theme::TAB_MAX_WIDTH, pinned_width: 36.0, fade_width: 24.0 })
        .viewport_width(cx.layout_size("top").map(|z| z[0]))
        .drag_state(&s.tabs, |s: &mut App| &mut s.tabs)
        .close_icon(k.glyph.close)
        .trailing_action(plus)
        .build_layered(k, layer);
    let diff = Button::new("+120  \u{2212}126  9 files").variant(ButtonVariant::Ghost).build(k);
    shell::titlebar(
        k,
        lights(),
        strip,
        vec![
            diff,
            icon_button_k(k, &SPLIT, "Split terminal"),
            icon_button_k(k, &SEARCH, "Search"),
            icon_button_k(k, &PANEL_RIGHT, "Hide files"),
        ],
    )
}

fn icon_button_k(k: &Kit, i: &'static Icon, label: &str) -> El {
    IconButton::new(i).variant(IconButtonVariant::GhostMuted).control_size(ControlSize::Large).label(label.to_owned()).build(k)
}

fn section_head(k: &Kit, title: &str, action: Option<(&'static Icon, &str)>) -> El {
    let r = row().w_full().h(28.0).pl(10.0).pr(2.0).center_y().child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_faint)).child(spacer());
    match action {
        Some((i, label)) => r.child(icon_button_k(k, i, label)),
        None => r,
    }
}

fn agent_row(k: &Kit, title: &str, meta: &str, age: &str, lead: El, selected: bool, depth: u8, height: f32) -> El {
    SidebarRow::new(title.to_owned())
        .avatar(lead)
        .subtitle(meta.to_owned())
        .subtitle_style(theme::META)
        .title_style(theme::BODY)
        .detail(age.to_owned())
        .height(height)
        .depth(depth)
        .selected(selected)
        .build(k)
        .key(title.to_owned())
}

fn project_row(k: &Kit, name: &str, meta: &str, dim: bool) -> El {
    let mut r = SidebarRow::new(name.to_owned()).icon(&FOLDER).detail(meta.to_owned()).height(theme::ROW_HEIGHT).title_style(theme::BODY_STRONG);
    if dim {
        r = r.title_style(theme::BODY);
    }
    r.build(k).key(name.to_owned())
}

fn server_row(k: &Kit, name: &str, meta: &str, i: &'static Icon, selected: bool) -> El {
    SidebarRow::new(name.to_owned()).icon(i).detail(meta.to_owned()).height(theme::ROW_HEIGHT).title_style(theme::BODY).selected(selected).build(k).key(name.to_owned())
}

fn left(k: &Kit) -> El {
    let t = tokens();
    let working = |color: Color| glyph("⠋", 17.0, color);
    let sidebar = Sidebar::new()
        .label("Projects")
        .inset(0.0)
        .row_gap(2.0)
        .header(section_head(k, "Projects", Some((&PLUS, "Open a folder"))))
        .child(project_row(k, "contr", "main", false))
        .child(agent_row(k, "Agent board polish", "opus", "14m", claude(k, 16.0, false), true, 1, 48.0))
        .child(agent_row(k, "Agent rows on ui.List", "b-241  sonnet", "6m", working(t.ink_100), false, 2, 44.0))
        .child(agent_row(k, "Panel PR header", "b-268  sonnet", "11m", working(t.ink_100), false, 2, 44.0))
        .child(agent_row(k, "Checks on hc event", "b-250  sonnet", "2m", working(t.blocked), false, 2, 44.0))
        .child(el().h(10.0))
        .child(project_row(k, "example", "main", false))
        .child(agent_row(k, "Asset pipeline", "opus  idle", "1h", claude(k, 16.0, true), false, 1, 48.0))
        .child(agent_row(k, "Sprite atlas rebuild", "b-88  deepseek  CI", "25m", glyph("⠶", 17.0, t.ink_30), false, 2, 44.0))
        .child(el().h(10.0))
        .child(project_row(k, "closed", "nothing open", true))
        .footer(section_head(k, "Servers", Some((&PLUS, "Add a server"))))
        .footer(server_row(k, "this Mac", "local", &LAPTOP, true))
        .footer(server_row(k, "linux-pc", "online", &SERVER, false))
        .footer(server_row(k, "build-box", "offline", &SERVER, false))
        .footer(row().w_full().h(1.0).fill(k.c.border_muted).mt(6.0))
        .footer(row().w_full().pt(8.0).pr(2.0).child(spacer()).child(icon_button_k(k, &GEAR, "Settings")))
        .build(k);
    shell::sidebar_panel(k).padding(10.0).child(sidebar)
}

enum Tone {
    Ink,
    Dim,
    Red,
}

fn terminal(k: &Kit) -> El {
    let t = tokens();
    let line = |parts: &[(Tone, &str)]| {
        let mut r = row().h(theme::LINE_H).center_y();
        for (tone, s) in parts {
            let color = match tone {
                Tone::Ink => t.terminal_text,
                Tone::Dim => t.ink_42,
                Tone::Red => t.blocked,
            };
            r = r.child(text((*s).to_owned()).font(theme::TERMINAL).color(color));
        }
        r
    };
    let blank = || el().h(theme::LINE_H);
    let dot = |rest: &'static str| line(&[(Tone::Ink, "\u{25cf} "), (Tone::Ink, rest)]);
    let lines = col()
        .child(dot("spawn (contr)(worker: \"b-241\", profile: \"builder\", task: \"#241 agent rows on ui.List\")"))
        .child(line(&[(Tone::Dim, "  \u{23bf}  started b-241 in hc terminal 17")]))
        .child(blank())
        .child(dot("spawn (contr)(worker: \"b-268\", profile: \"builder\", task: \"#268 panel PR header\")"))
        .child(line(&[(Tone::Dim, "  \u{23bf}  started b-268 in hc terminal 18")]))
        .child(blank())
        .child(dot("b-250 reported: blocked at a permission prompt"))
        .child(line(&[(Tone::Dim, "    (wants to run go test -tags hccore ./internal/hc/...)")]))
        .child(blank())
        .child(dot("Three workers are out:"))
        .child(line(&[(Tone::Ink, "    b-241  #241 agent rows on ui.List      building")]))
        .child(line(&[(Tone::Ink, "    b-268  #268 panel PR header            building")]))
        .child(line(&[(Tone::Ink, "    b-250  #250 checks on hc event         "), (Tone::Red, "blocked, needs your OK")]))
        .child(blank())
        .child(line(&[(Tone::Ink, "  b-268 and b-250 both touch internal/app/panel.go. I'll merge")]))
        .child(line(&[(Tone::Ink, "  #268 first and have b-250 merge main before review.")]))
        .child(blank())
        .child(line(&[(Tone::Dim, "\u{273b} Waiting for reports\u{2026} (3 workers)")]));
    let prompt = row()
        .w_full()
        .mt(14.0)
        .px(12.0)
        .py(9.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, t.tick)
        .child(text("> ").font(theme::TERMINAL).color(t.ink_95))
        .child(el().size(8.0, 17.0).fill(t.ink_95));
    let hint = text("  \u{23f5}\u{23f5} auto mode on (shift+tab to cycle)").font(TextStyle::new(12.0, 400.0).mono().lh(20.0)).color(t.ink_42);
    // The orchestrator's message rail: a count and seven ticks, down the right edge.
    let tick = |w: f32, on: bool| el().w(w).h(2.0).radius(1.0).fill(if on { t.ink_100 } else { t.tick });
    let rail = col()
        .items_end()
        .gap(9.0)
        .child(text("3").font(theme::COUNT).color(t.ink_100))
        .children([8.0, 12.0, 8.0, 14.0].map(|w| tick(w, false)))
        .children([10.0, 12.0, 8.0].map(|w| tick(w, true)));
    shell::terminal_panel(k)
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(row().w(1.0).h(1.0))
                .child(col().grow(1.0).h_full().pt(14.0).px(20.0).pb(12.0).child(spacer()).child(lines).child(prompt).child(hint))
                .child(col().h_full().pr(10.0).center_y().child(rail)),
        )
}

fn files(s: &App, k: &Kit) -> El {
    let t = tokens();
    let style = {
        let mut st = TreeStyle::from_kit(k).label(theme::ROW_SMALL, t.ink_50, t.ink_100).hover(t.hover);
        st.row_height = theme::FILE_ROW_HEIGHT;
        st.indent = 21.0;
        st.selected = Color::TRANSPARENT;
        st
    };
    let (rest, strong, faint) = (t.ink_50, t.ink_90, t.ink_24);
    let tree = Tree::new(|s: &App| &s.files, |s: &mut App| &mut s.files, |n: &FileNode<Change>| n.name.as_str())
        .name("contr files")
        .style(style)
        .row(move |n, r| {
            let item = TreeItem::file(n, r);
            match n.data {
                Some(Change { added, removed, kind }) => {
                    let item = match kind {
                        Kind::Modified | Kind::New => item.label_color(strong),
                        Kind::Deleted => item.label_color(faint).icon_color(faint),
                        Kind::Plain => item.label_color(rest),
                    };
                    if added + removed > 0 { item.changes(added, removed) } else { item }
                }
                None => item,
            }
        })
        .build(s, k)
        .grow(1.0)
        .min_h(0.0);
    let meter = |label: &str, value: f32| Progress::new(Some(value)).label(label.to_owned()).show_value().build(k);
    let meters = col()
        .w_full()
        .pt(12.0)
        .px(10.0)
        .gap(12.0)
        .child(meter("Context", 0.45))
        .child(meter("Claude", 0.38))
        .child(meter("Codex", 0.12));
    shell::files_panel(k)
        .padding(10.0)
        .child(section_head(k, "contr", Some((&FILTER, "Show changed files only"))))
        .child(tree)
        .child(row().w_full().h(1.0).fill(k.c.border_muted))
        .child(meters)
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = if s.contr { contr_kit() } else { Kit::neutral(true) };
    let mut layer = Vec::new();
    let bar = titlebar(s, cx, &k, &mut layer);
    shell::window(&k, bar, left(&k), terminal(&k), files(s, &k), layer)
}

fn main() {
    rune::App::new(AppSpec {
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        ..AppSpec::new("kit preview", (1710.0, 1112.0), init, view)
    })
    .run();
}
