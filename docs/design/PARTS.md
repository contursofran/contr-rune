# Build by parts

contr-rune follows the "contr · from zero" design (dark theme first). We build it one part at a time.
Each part is one small PR. A part is done when it **looks** like the reference and **works** like it.

## Reference

| File | What |
|---|---|
| `main.html` | Board "1 · Orchestrator selected", static HTML. Exact sizes, colours and fonts are in its CSS. |
| `main.png` | `main.html` rendered at 1710×1112 (headless Chrome). Compare a `--snap` of the app with it. |
| `tokens.css` | All design tokens: dark and light colours, spacing, radius, sizes, type styles. |

Source artifacts (owner's claude.ai): canvas "contr · from zero" (`45a3e113-1779-433f-b075-a548b2c4f0da`)
and design system "contr" (`18e5fc02-6836-4af3-8cfd-d09fdfb8c48e`, one README and preview per component).

To re-render `main.png`:

```sh
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --hide-scrollbars \
  --force-device-scale-factor=1 --window-size=1710,1112 --virtual-time-budget=5000 \
  --screenshot="$PWD/docs/design/main.png" "file://$PWD/docs/design/main.html"
```

## Rules for every part

- Use token names from `src/theme.rs`, never raw colours in views.
- No amber or green in the chrome. Red (`blocked`) only means "needs you".
- Check the result with `cargo run -- --snap out.png --scene <scene>` at 1710×1112 and compare it with
  `main.png`. Put both images (or a crop of the part) in the PR description.
- Keep what already works (PTY, keys, scrollback) working.
- Each part adds stable keys on its elements and a headless test in `tests/` that checks its bounds
  (`--tree`) and its behaviour (`--script`), like `tests/tabs.rs`, plus the `--snap` image in `docs/design/snaps/`.

## Parts, in order

| # | Part | Design components | Done when |
|---|---|---|---|
| 1 | Tokens and window shell | Window, Panel, TitleBar (left area only) | Theme has every dark token and type style; Inter is bundled; the window is glass (or its opaque composite), radius 14; 38 px title bar with room for the traffic lights; three panels (sidebar 334 / files 302 outer, 312 / 280 content; terminal in between) with gap 8, radius 12, `panel` fill and `panel-line` border. Existing content still works inside them. |
| 2 | Tabs | Tab, IconButton | Tab strip in the title bar from x=350: height 30, max 180, 12.5 px Inter, selected = panel fill + inset line, hover fill, `×` 16 px on the selected tab, `+` 26 px button. Click, close, new, keys work. |
| 3 | Terminal panel | Terminal | Padding 14/20/12, `terminal` type 13/20, `terminal-text` ink, ANSI palette readable on the panel, cursor like the design. |
| 4 | Status glyph | StatusGlyph | Braille glyph: working (10 frames in 0.9 s), waiting (⠿ pulse 1.6 s), idle (⠶ at `ink-30`), needs-you (red). Animates only while on screen and working. Used in tabs. |
| 5 | Sidebar projects | ProjectRow, AgentRow | "Projects" head with `+`; project row (folder icon, name 500 weight `ink-85`, branch meta); its terminals as rows under it (glyph, title, mono meta, age), rail line, selected row = `ink-100` with no fill. |
| 6 | Sidebar foot | ServerRow | "Servers" head, "this Mac · local" row, hairline, settings gear 24 px. |
| 7 | Title bar right | DiffSummary, IconButton | `+120 −126 9 files` mono summary from git; split, search and files-toggle icon buttons; files toggle hides the files panel. |
| 8 | Files panel | FileTree, UsageMeter | Git tree of the selected project: folder/file icons, 26 px rows, indent rails, modified/new/deleted styles with `+a −r`; meters at the bottom. |
| 9 | Later | Outline, QuestionsNotch, QuestionsCard, Settings, light theme | Need agent data (hc) or later design boards. |
