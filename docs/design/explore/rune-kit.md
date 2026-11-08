# Exploration: contr's main screen on rune-kit

`examples/kit_preview.rs` builds `docs/design/main.html` with static fake data from rune-kit
(same pinned Rune rev `a7f276c`, as a dev-dependency; `src/` is untouched). One view tree,
two kits:

| Image | Kit |
|---|---|
| `kit-neutral.png` | `Kit::neutral(true)`, as shipped |
| `kit-contr.png` | `Kit::neutral(true)` with contr's tokens (`src/theme.rs`) written into `Palette`, `Type` and `Radius` |

Reference: `docs/design/main.png`. Render (1710x1112, scale 1):

```sh
cargo run --example kit_preview -- --snap docs/design/explore/kit-contr.png   --size 1710x1112 --scale 1 --theme dark contr
cargo run --example kit_preview -- --snap docs/design/explore/kit-neutral.png --size 1710x1112 --scale 1 --theme dark neutral
```

The kit's own window theme (`rune-theme-opencode`) is not used; only the neutral kit and our
tokens. Both renders use contr's bundled Inter and JetBrains Mono, so the difference between
the two images is the kit's tokens, not the fonts.

## Mapping

"Closeness" is for `kit-contr.png` against `main.png`: **close** = same at a glance,
**near** = same job, visible differences, **far** = rebuilt by hand.

| Design component | rune-kit component used | Closeness | What cannot be matched, and why |
|---|---|---|---|
| Window / panels (3 cards, gap 8, radius 12, `panel` fill + line) | none, custom (`col().fill().radius().border()`); `Kit` supplies `bg_base`, `border_muted`, `radius.lg` | close | `SplitView` was not used (see App shell). Glass blur and the three glows are not in rune-kit or in the example; the window is the opaque composite (`window_solid`). |
| App shell (titlebar + sidebar + main + inspector) | `WindowTitlebar` only | near | `SplitView` has a fixed 48 pt toolbar band (`TOOLBAR_HEIGHT`, not a setting), the sidebar is a flush pane (not a rounded card), the main pane is the card, and the sidebar toggle is built in. contr's bar is 38 pt and all three panels are cards. Using `SplitView` would give resize, collapse and peek for free, at the cost of the 48 pt band. |
| Title bar, traffic lights | `WindowTitlebar` (`height(38)`, leading/tabs/trailing slots) + three custom dots | close | Real lights come from the window chrome; the dots are only drawn for the snapshot. |
| Tab (selected = panel fill + inset line, 30 high, max 180, `x`) | `TabStrip` + `TabItem` (avatar, icon, close icon, `TabOverflow` max 180) | near | Tab height, radius and the active fill/ring are fixed in the component, not tokens: the selected tab is a 28-ish pt pill with the kit's fill, not contr's `panel` + 6.5 % inset line. Tabs are placed on an even grid with 1 px separators between inactive tabs, and `+` sits after the strip's slot, not hugging the last tab at 2 px. Strip starts at x=342, design at 352. Busy glyph works as an avatar (static braille here; no animation was wired). |
| IconButton (28 px, radius 8, 50 % ink, hover fill) | `IconButton` (`GhostMuted`, `ControlSize::Large`) | close | Size ladder is fixed (Small 20, Large 28, XL 36); contr's 24 and 26 px buttons snap to the nearest. Icons are our own SVGs via `Icon`. |
| Diff summary `+120 −126 9 files` (mono, two tones) | `Button` (`Ghost`), one label | near | A button label is one run in the kit font: no mono face, no dim `−126`. Needs `label_with` or a custom row. |
| Section head ("Projects", "Servers" + `+`) | none, custom row + `IconButton` | near | `SidebarSection` / `SidebarHeader` exist, but their header is a 28 pt collapsible heading with counts and a disclosure triangle; contr's is a 12 px / 500 label that never collapses. Used `Sidebar::header` / `footer` with a hand-made row. |
| Project row (folder, name 500, branch meta) | `SidebarRow` (`icon`, `detail`, `title_style`) | close | Row height and radius are builder options; `title_style` gives the 500 weight. Detail is 11 px faint text, not the project row's `ink-26`. |
| Agent row (glyph, title, mono meta, age; rail line) | `SidebarRow` (`avatar`, `subtitle`, `subtitle_style`, `detail`, `depth`, `height`) | near | Two-line layout works, but the age sits vertically centred, not on the meta line. No indent rail line. Subtitle is one string (the three meta fields are joined with spaces, no `margin-left: auto`). Selected row gets a fill; contr's has none, only `ink-100` text (the fill is `Palette::pressed`, shared with the pressed state). Glyph column is 16 pt at the row's left padding, so depth steps are 16 pt, not 17 + 9. |
| Status glyph (braille, claude spark, red variant) | none, custom (`text` in `avatar`, `icon` for the spark) | far | rune-kit has `Spinner`, `Loader`, `ProgressCircle` and `RowStatus`, but no braille or glyph-cell indicator, and none of them animate in a snapshot. The 10-frame braille cycle, the pulse and the 2.4 s spark spin would be custom animation either way. |
| Server row (icon, name, state) | `SidebarRow` (`icon`, `detail`) | close | Same limits as the project row. |
| Settings footer (hairline, 24 px gear) | custom hairline + `IconButton` | close | `ControlSize` has no 24 px step (renders 28). |
| Terminal panel (text, prompt box, hint) | none, custom (mono `text` lines) | n/a | rune-kit has no terminal. The real grid stays in `src/grid.rs`. |
| Orchestrator tick rail (count + ticks) | none, custom | n/a | Not a kit concept. |
| Files tree (rows 26, folders, rails, chevrons) | `FileTree` + `Tree` + `TreeItem` + `TreeStyle` | near | Structure, folders-first order, indent guides, open/close, keyboard and virtualisation come free. Differences: chevrons always show (only `chevron_only_folders` exists, no "none"); file icons are the kit's (`FILE_CODE`, `FILE_TEXT`), not Go/Markdown glyphs; guide positions are fixed; no strike-through for deleted rows (dim colour only); rows can be selected (cursor) which contr's tree does not have. |
| Change counts `+38 −12` | `TreeItem::changes(added, removed)` | close | Colours come from `success.icon` and `danger.icon` (set to ink and dim ink for contr; kit-neutral shows green and red). The kit draws the minus as `−` in the same weight; contr's is mono 11 px. |
| Modified / new / deleted row styles | `TreeItem::label_color` / `icon_color` | near | Colour only. Modified and new look the same (contr also uses the same ink for both). Deleted has no line-through. |
| Usage meter (label, value, 4 px bar) | `Progress` (`label`, `show_value`) | near | Track is a fixed 8 px pill with a 1 px border; contr is 4 px, 2 px radius, no border. Value text is `NN%` only: `90k of 200k  45%` is not possible. Label/value are 13 px kit text (`text_small`), contr's are 12 px label + mono 11.5 px value. |
| Badge, Divider | not used | n/a | Nothing in main.png needs them (row counts are plain text). `Badge` would suit later parts (unread count). |

## Token-level gaps (apply everywhere)

- `Kit` colours: contr's translucent ink ramp (`ink-26` ... `ink-62`) is 15 steps; the kit has
  about 6 text/icon roles (`base`, `muted`, `faint` plus `weak`/`strong` for the older
  components). The ramp collapses onto them; the rest has to be explicit colours in views.
- Components that read old-style roles (`Progress`, `Button`) use `surface_*`, `std_*` and
  `border_active`, a second set of tokens to fill.
- Type: `ty.ui`, `small`, `caption()` and friends are fixed names; contr's 12.5/18, 11 mono and
  11.5 mono styles map onto them only by overriding row by row (`title_style`, `subtitle_style`).
- Radius: five steps (`xs..xl`); contr's `1, 2, 4, 6, 7, 8, 9, 12, 14` collapse onto them.
- Motion, shadows, depth sheen and focus rings come from the kit and were not changed.

## Recommendation

Use rune-kit for the parts where behaviour is the cost and the look is flexible:

- **Part 5 / 6 (sidebar):** `Sidebar` + `SidebarRow`. Keyboard list (arrows, type-ahead, Return),
  roving focus, accessibility roles, reorder-by-drag and rename in place are all free, and the
  look is within a few points. Keep the section heads and the status glyph custom.
- **Part 8 (files):** `FileTree` + `Tree` (+ `TreeItem::changes`). Virtualised, keyboard-driven
  and with guides. Accept the chevrons, or ask upstream for a "no chevron" option. Meters:
  keep custom (a 4 px bar is ten lines; `Progress` cannot be made 4 px or show `90k of 200k`).
- **Part 2 (tabs):** `TabStrip` is the biggest saving (reorder, overflow scroll, close, busy
  status, tear-out) but its tab chrome is not themable to contr's spec. Take it if the owner is
  fine with the kit's tab shape; otherwise stay custom.
- **Part 7 (icon buttons):** `IconButton` (28 px) is fine; the summary should be a custom row.

Keep custom: the window and panel cards (`SplitView`'s 48 pt band and flush-pane model do not
fit; revisit if `TOOLBAR_HEIGHT` and the pane style become settings), the terminal and its
tick rail, the status glyph (braille cycle, spark spin), the diff summary, section heads and
the usage meters.

A mixed approach is cheap: the example already does it with one `Kit` value, and every kit
component takes the same tokens, so kit-built and custom parts stay in step by reading the
same `Kit`. The remaining cost is that contr's tokens (`src/theme.rs`) must be mapped into a
`Kit` once (`contr_kit()` in the example is that mapping, about 40 lines).
