# AGENTS.md

Rules for agents that work in contr-rune. Read this file first.

## What this is

contr-rune is a Rust app on [Rune](https://github.com/anomalyco/rune), a GPU UI framework (pinned rev in
`Cargo.toml`). It shows projects, terminal tabs and a real PTY terminal. We rebuild its look and behaviour to
match the "contr · from zero" design.

- Toolchain: Rust 1.99.1 (`rust-toolchain.toml`). Cargo goes through mbx (a build cache): start each shell
  command with `export PATH="$HOME/Library/Application Support/mbx/bin:$PATH:$HOME/.cargo/bin"` and run builds and
  tests at low priority (`nice -n 10 cargo …`). Run only one cargo command at a time, in the foreground; never run
  several `cargo test`s in parallel.
- Rune source: `~/.cargo/git/checkouts/rune-*/<rev>/`. Read `crates/rune/README.md` there before you use a
  Rune feature you do not know.
- Rune examples: `examples/` in that checkout (online: https://github.com/anomalyco/rune/tree/dev/examples;
  the checkout matches our pinned rev, `dev` can be newer). Find a working example before you build a
  pattern from zero. Start points: `hello` (headless `--snap`/`--scene`), `kit-gallery` (rune-kit widgets),
  `settings` and `cursed-settings` (rows, toggles), `glass-panel`, `glass-card` and `frosted-bar` (glass),
  `overlay` (floating parts), `shapes` and `sketch` (paths and icons), `todo` (lists and input).
- Rust style: before you write or change Rust, read the `rust-best-practices` skill
  (`~/.claude/skills/rust-best-practices/SKILL.md`, and its `references/` when a topic needs depth) and follow it.
  Before you open a PR, check your diff against the `rust-review` skill (`~/.claude/skills/rust-review/SKILL.md`)
  and fix what it finds. Install both with `npx skills add morganlinton/BeautifulRust -g -a claude-code`.

## Design source

The app is built from **rune-kit** components on rune-kit's **neutral theme** (owner, 2026-11-08). The
"contr · from zero" design is the guide for layout, content and behaviour, not for exact colours.

- `docs/design/explore/kit-neutral.png`: the visual target (board 1N, 1710×1112).
- `examples/kit_preview.rs`: the code that renders it. Start each part from the matching piece there.
- `docs/design/explore/rune-kit.md`: which rune-kit component maps to which design component.
- `docs/design/PARTS.md`: the build order and the "done when" for each part.
- `docs/design/main.png` and `main.html`: the original design. Use them for layout, copy and behaviour.
- rune-kit docs: `crates/rune-kit/README.md` and `crates/rune-kit/examples/` in the Rune checkout.

## Work by parts

- Build one part of `PARTS.md` at a time. One part = one small PR. Do not start the next part in the same PR.
- Branch `part-<n>-<name>` off `origin/main`.
- The part must **look** like the reference and **work** like it. Keep what works (PTY, keys, scrollback,
  close on exit).

## Stay in scope

- Do only what the brief asks. No extra features, refactors, renames, reformatting or "while I am here" fixes.
  Write other ideas in the PR under "Not done" and do not build them.
- No speculative code: no options, parameters, traits, modules or files that nothing uses yet. Delete code your
  change makes dead.
- If the brief is unclear, or the plan turns out wrong, stop and report it. Do not widen the task to work around it.
- Keep the PR small: about 400 changed lines of code at most, not counting tests, fixtures and snaps. If the part
  needs more, stop and propose a split.

## Look

- Use a rune-kit component wherever one exists. Build a custom element only when rune-kit has none (for
  example the terminal grid), and say why in the PR.
- Colours, type and radius come from the `Kit` (`Kit::neutral`). No raw colours in views. Accept the kit's
  shapes and colours (tab chrome, meter size, green/red change counts); do not fight them.
- Decisions already made (do not reopen):
  - Shell: `WindowTitlebar` (38) and three panel cards, as in `kit_preview.rs` (not `SplitView`).
  - The window stays native (macOS traffic lights). Real glass is a later part.
  - Mono font fallback keeps the Nerd Font icons: `JetBrains Mono, Symbols Nerd Font Mono, JetBrainsMono Nerd Font, ui-monospace`. "Symbols Nerd Font Mono" (MIT) is bundled in `assets/fonts`, so prompt icons draw on any machine.

## Check your work headlessly

Rune runs headless. Every part must be checked in three ways, not by eye alone:

1. **Image:** `cargo run -- --snap <out.png> --size 1710x1112 --scale 1 --scene three-tabs --at 1`. Compare it
   with `docs/design/explore/kit-neutral.png`. Commit it as `docs/design/snaps/part<n>.png`.
2. **Element tree:** give the part's elements stable keys (for example `tab/0`, `tab/0/close`). Run with
   `--tree <out.json>` and check the bounds against the target (run `kit_preview` with
   `--tree` for its numbers).
3. **Behaviour:** drive the part with `--script <script.json>` (click, hover, keys) and check the result in
   the `--tree` output.

Make 2 and 3 a lasting integration test under `tests/`, so a later part cannot break this part silently.

## Tests: few, fast, useful

- Each test protects one behaviour that a user or a later part depends on. Do not test the type system,
  constants, simple getters, Rune or rune-kit behaviour, or something another test already covers.
- Prefer a unit test of a pure function. Use a headless `--script`/`--tree` test only for what needs the UI
  (bounds, clicks, keys). One headless test per behaviour, not per variant: put variants in a table in a unit test.
- Use a real PTY or shell only when the test is about the PTY. Otherwise use a fake terminal or the fake hc.
- Budget: a unit test under 50 ms, a headless test under 3 s, a whole new test file under 15 s. A slower test
  needs a reason in the PR.
- No `sleep` and no polling loops. Wait on a state (the tree, the fake log) and keep script end times short.
- When behaviour changes, change or delete the old tests. Never keep tests for behaviour that is gone.
- While you work, run only the test file you touch. Run the full suite once, before the PR, through the shared
  lock so two agents never run tests at the same time:
  `flock /tmp/contr-rune-tests.lock nice -n 10 cargo test` (`flock` from Homebrew: `brew install flock`).

Before you open the PR: `cargo build` with no warnings and `cargo test` pass.

## Pull requests

- The PR description says what matches the design, what does not, and why (for example a Rune limit). It
  holds the tree numbers against the spec.
- No `Co-Authored-By`, AI attribution or "Generated with" lines in commits or PRs.
- Flow: builder opens the PR → the owner runs it in a real window → fixes → merge. No reviewer agent in this repo (owner, 2026-11-08): the headless `--snap`, `--tree` and `--script` checks and tests replace it.

## hc

- Read `docs/design/HC-PLAN.md` before any H part. Its Contract section is the source of truth for methods, labels, keys and fixtures.
- Headless runs (`--snap`, `--tree`, `--script`) and tests never dial a real hc unless `CONTR_HC_FAKE` is set (the fake core is in-process).
- Never start, stop, kill or change the owner's real hc terminals from tests or agents. A test that needs a real core uses its own `HC_SESSION` and scratch dirs.
