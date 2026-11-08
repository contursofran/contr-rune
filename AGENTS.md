# AGENTS.md

Rules for agents that work in contr-rune. Read this file first.

## What this is

contr-rune is a Rust app on [Rune](https://github.com/anomalyco/rune), a GPU UI framework (pinned rev in
`Cargo.toml`). It shows projects, terminal tabs and a real PTY terminal. We rebuild its look and behaviour to
match the "contr · from zero" design.

- Toolchain: Rust 1.99.1 (`rust-toolchain.toml`). `cargo` is not on PATH: start each shell command with
  `. "$HOME/.cargo/env"`.
- Rune source: `~/.cargo/git/checkouts/rune-*/<rev>/`. Read `crates/rune/README.md` there before you use a
  Rune feature you do not know.
- Rune examples: `examples/` in that checkout (online: https://github.com/anomalyco/rune/tree/dev/examples;
  the checkout matches our pinned rev, `dev` can be newer). Find a working example before you build a
  pattern from zero. Start points: `hello` (headless `--snap`/`--scene`), `kit-gallery` (rune-kit widgets),
  `settings` and `cursed-settings` (rows, toggles), `glass-panel`, `glass-card` and `frosted-bar` (glass),
  `overlay` (floating parts), `shapes` and `sketch` (paths and icons), `todo` (lists and input).

## Design source

- `docs/design/PARTS.md`: the build order, the "done when" for each part, and the rules below in detail.
- `docs/design/main.png`: the target (1710×1112). `docs/design/main.html`: the exact CSS values.
- `docs/design/tokens.css`: all tokens (dark and light).
- The owner's artifacts: canvas "contr · from zero" and design system "contr" (one README per component).

## Work by parts

- Build one part of `PARTS.md` at a time. One part = one small PR. Do not start the next part in the same PR.
- Branch `part-<n>-<name>` off `origin/main`.
- The part must **look** like the reference and **work** like it. Keep what works (PTY, keys, scrollback,
  close on exit).

## Look

- Colours, sizes and type styles come only from `src/theme.rs` token names. No raw colours in views.
- No amber or green in the chrome. Red (`blocked`) means only "needs you".
- Decisions already made (do not reopen):
  - Panel widths follow the rendered reference: sidebar 334 and files 302 outer (312 / 280 content). The
    sidebar's right edge is at x=342; the terminal panel and the tab strip start at x=350.
  - The window stays native (macOS traffic lights) with the opaque glass colour. Real glass is a later part.
  - Mono font fallback keeps the Nerd Font: `JetBrains Mono, JetBrainsMono Nerd Font, ui-monospace`.

## Check your work headlessly

Rune runs headless. Every part must be checked in three ways, not by eye alone:

1. **Image:** `cargo run -- --snap <out.png> --size 1710x1112 --scale 1 --scene three-tabs --at 1`. Compare it
   with `docs/design/main.png`. Commit it as `docs/design/snaps/part<n>.png`.
2. **Element tree:** give the part's elements stable keys (for example `tab/0`, `tab/0/close`). Run with
   `--tree <out.json>` and check the exact bounds against the design values.
3. **Behaviour:** drive the part with `--script <script.json>` (click, hover, keys) and check the result in
   the `--tree` output.

Make 2 and 3 a lasting integration test under `tests/`, so a later part cannot break this part silently.
If the PTY makes a test flaky, use a scene with fake terminals.

Before you open the PR: `cargo build` with no warnings and `cargo test` pass.

## Pull requests

- The PR description says what matches the design, what does not, and why (for example a Rune limit). It
  holds the tree numbers against the spec.
- No `Co-Authored-By`, AI attribution or "Generated with" lines in commits or PRs.
- Flow: builder opens the PR → reviewer checks it → fixes → the owner runs it in a real window → merge.
