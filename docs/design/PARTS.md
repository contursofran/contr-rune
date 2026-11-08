# Build by parts

contr-rune is built from **rune-kit** components on rune-kit's **neutral theme** (owner, 2026-11-08). We
build it one part at a time. Each part is one small PR. A part is done when it **looks** like the target and
**works** like the design.

## Reference

| File | What |
|---|---|
| `explore/kit-neutral.png` | **The visual target** (board 1N): the main screen from rune-kit on `Kit::neutral(true)`, 1710×1112. |
| `../../examples/kit_preview.rs` | The code that renders the target, with static data. Start each part from its matching piece. |
| `explore/rune-kit.md` | Design component → rune-kit component, and what the kit cannot do. |
| `explore/kit-contr.png` | The same tree with contr's tokens (board 1K). Not the target; kept for comparison. |
| `main.html`, `main.png` | The original design (board 1). Use it for layout, copy and behaviour, not for colours. |
| `tokens.css` | The original design tokens. |

Render the target and its element tree:

```sh
cargo run --example kit_preview -- --snap docs/design/explore/kit-neutral.png --size 1710x1112 --scale 1 --theme dark neutral
cargo run --example kit_preview -- --tree /tmp/kit-tree.json --size 1710x1112 --scale 1 --theme dark neutral
```

Source artifacts (owner's claude.ai): canvas "contr · from zero" (`45a3e113-1779-433f-b075-a548b2c4f0da`;
boards 1K and 1N are the rune-kit renders) and design system "contr" (`18e5fc02-6836-4af3-8cfd-d09fdfb8c48e`).

## Rules for every part

- Use a rune-kit component wherever one exists; custom only when the kit has none, with the reason in the PR.
- Colours, type and radius come from the `Kit`. No raw colours in views. Accept the kit's shapes and colours.
- Give the part's elements stable keys. Check it headlessly: `--snap` against `kit-neutral.png`, `--tree`
  bounds against the target's tree, `--script` for clicks and keys. Make the tree and script checks a lasting
  test under `tests/`. Commit the snap as `snaps/part<n>.png`.
- Keep what already works (PTY, keys, scrollback, close on exit) working.

## Parts, in order

Part 1 of the first plan (custom tokens and window shell, PR #1) is merged. The custom tab strip (PR #3) was
closed for this plan.

| # | Part | rune-kit | Done when |
|---|---|---|---|
| K1 | Kit foundation and shell | `Kit::neutral`, `WindowTitlebar` | rune-kit is a normal dependency; the app builds one `Kit::neutral(true)` and every view reads it; the custom colour tokens in `src/theme.rs` are gone (keep sizes, fonts and the terminal palette); shell = `WindowTitlebar` 38 + three panel cards as in `kit_preview.rs`. Existing sidebar, tabs and terminal still work inside it. |
| K2 | Tabs | `TabStrip`, `TabItem`, `TabOverflow` | Tabs of the selected project in the title bar: icon, program label, close, `+`, busy status on a working terminal, overflow. Click, close, new, reorder and the keys work. |
| K3 | Terminal panel | custom grid, kit colours | Terminal text and background from the kit; padding as the target; cursor visible; ANSI palette readable on the kit panel. |
| K4 | Sidebar | `Sidebar`, `SidebarRow`, `SidebarSection` or header slot, `IconButton`, kit `Spinner`/`RowStatus` | "Projects" head with `+`; project rows; terminal rows under each project (depth, label, meta, age, busy status); "Servers" with "this Mac · local"; settings gear in the foot. Keyboard list, select, open a terminal. |
| K5 | Title bar right | `Button` (ghost), `IconButton` | Diff summary of the selected project from git (`+a −r n files`); split, search and files-toggle buttons; files toggle hides the files panel. |
| K6 | Files panel | `FileTree`, `Tree`, `TreeItem::changes`, `Progress` | Git tree of the selected project with change counts and modified/new/deleted styles; open/close folders; meters at the bottom. |
| K7a | Settings and light theme | `SegmentedControl`, `Button`, modal overlay, `Kit::neutral(false)` | Gear in the sidebar foot and ⌘, open a modal (esc, Done, scrim close it): Appearance System/Dark/Light, versions (contr-rune, hc "not installed"). The whole app, terminal included, follows the choice live; saved in `settings.json`. Done (Transparency and Updates are left out: opaque window, no updater). |
| K7b | Later | `Popover`, `TextInput`, `Switch`, `Select` | Questions notch and card, outline, and the rest of settings. Need agent data (hc) or later boards. |
