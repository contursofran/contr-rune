# PARITY-PLAN: contr-rune to feature parity with contr

Status: plan, 2026-11-09. Read-only research. Nothing was built, run or changed in either repo.

## 0. Which contr is "current" (read first)

The brief names the Swift app at `6be48b6` as the current contr. **It is not.**

| Line | Where | State |
|---|---|---|
| Swift (tmux runtime, board, slots, checks) | local checkout `~/…/contr`, branch `contur/hopeful-dirac-konwor`; `6be48b6` | Not on GitHub (`gh api …/commits/6be48b6` → 422). Not an ancestor of the local `origin/main`. Last commit 2026-10-06. Its `herdr-migration.md` (P34) is a proposal only. |
| Rust/GPUI | local stale `origin/main` = `fae6154` (2026-10-24); GitHub tag `rust-final` | Replaced by Go (Go D1, D2, D27). |
| **Go + MyGo, on hc** | **GitHub `contursofran/contr` main = `5d3ef7a` (2026-11-07), tags up to `v0.2.48`, primary language Go** | **Current.** README: "contr is a native macOS client for herdr-core (`hc`)". Same app that contr-rune's `HC-PLAN.md` calls "[go]" and copies its contract from. |

This plan therefore measures parity against **Go contr at `5d3ef7a`**. The Swift line's
herdr proposal is already done in Go (Go D28 "own orchestration on hc", D95 "no longer connects to herdr").
Swift-only features are listed in §1.3 as "not in current contr". If the coordinator really wants the Swift
feature set, this plan is the wrong one: see Q0.

Go sources were read with `gh api repos/contursofran/contr/contents/<path>?ref=5d3ef7a`. Citations:
`go:<path>` = that file; `D<n>` = Go `docs/DECISIONS.md`; `spec/<X>.md` = Go `docs/spec/<X>.md`;
`rune:<path>:<line>` = contr-rune at `ceedb62`.

## 1. Feature table (Go contr, built today → contr-rune)

Status: **done**, **partial**, **missing**, **n/a** (deliberate difference; not a gap).

### 1.1 UI features

| # | Feature in Go contr (built) | Go source | contr-rune | Rune source / note |
|---|---|---|---|---|
| 1 | Window: title bar row + three panel cards (sidebar, terminal, files) | spec/Window.md; D168, D177 | done | K1; rune:src/shell.rs:20-62 (38 pt bar, decided in AGENTS.md) |
| 2 | Glass window (vibrancy, transparent) + Settings Background/Panels transparency sliders | D158-D164, D162 | n/a (deferred) | AGENTS.md "Real glass is a later part"; rune:src/dialog.rs:6-7 |
| 3 | Themes System/Dark/Light, live, saved | D119 | done | K7a; rune:src/settings.rs:50-65 (own `contr-rune/settings.json`) |
| 4 | Project rows: folder icon, name, branch meta; empty project dim | spec/ProjectRow.md; D171 | done | K4; rune:src/sidebar.rs:89,133 |
| 5 | Projects persist across launches (`projects.json`: order, hidden, recent, folders) | go:internal/projects/store.go:41-55; D109, D116 | **missing** | Projects come from `CONTR_PROJECTS`, else cwd+home, plus agents' folders and `+` picks (rune:src/main.rs:151-165, 313-324, 335-356). Nothing is saved. |
| 6 | Launch selects first live orchestrator (D173); empty projects last (D174); click on selected project folds it (#344) | spec/ProjectRow.md | **missing** | rune selects project 0 and opens/restores tabs (rune:src/main.rs:675-694) |
| 7 | Projects `+` menu: "New orchestrator" (folder picker → spawn `orchestrator` profile) and "Open projects"; empty project menu "New orchestrator in …" | D57, D171; go:cmd/contr/neworch.go (spawnOrchestrator) | **missing** | rune `+` only opens a folder (rune:src/sidebar.rs:256-266) |
| 8 | Projects modal: filter, drag reorder, close (hide), recent + open, open folder, clone from GitHub | D110, D113, D116, D117; go:internal/app/projectsmodal.go | **missing** | — |
| 9 | Agent rows under project: orchestrator, workers on rail, glyph per state, line 2 name/model/effort/age, ages each minute | spec/AgentRow.md; D137, D169, D189 | done | H5, H6; rune:src/sidebar.rs:205-235, rune:src/hc/model.rs:109-235 |
| 10 | Agent row line 1 = GitHub issue title (`gh issue view`) | D174 | **missing** | rune uses terminal title/name (`display_title`, rune:src/hc/model.rs:61) |
| 11 | Agent row "CI" when the worker's PR has unfinished checks | D145 | **missing** | rune design draws it (`deepseek CI` in docs/design/main.png) |
| 12 | Agent row context menu: Interrupt, Stop, Open PR (`gh pr view --web`) | spec/AgentRow.md "Context menu"; go:internal/agents/agents.go:676 (`terminal.interrupt`) | **missing** | no `on_secondary_press` anywhere in rune:src |
| 13 | Foreign terminals: Claude in a plain terminal is a session; readable names (D156); grey Claude mark for a session that does not orchestrate (D191) | D149, D156, D190, D191 | partial | foreign Claude = orchestrator (rune:src/hc/model.rs:76,409-422); no grey mark, no D156 naming order |
| 14 | Selecting an agent shows its terminal, terminal takes focus | D80, D188 | done | H8; rune:src/main.rs:361-375 |
| 15 | Title bar tabs: Claude sessions of the project first, then selected worker, then plain terminals (one shared set, D190) | spec/TitleBar.md; D190 | partial | rune: tabs = project's plain terminals + agent tabs opened by selection (rune:src/main.rs:70-75, 960-1000) |
| 16 | `+`/⌘T new plain hc terminal; ⌘W closes plain terminal (kill + rm); agent tab has no close | D82, D85 | done | H3; rune:src/main.rs:409-430 (agent tab only detaches) |
| 17 | Tab middle-click close; tab context menu Close / Close others | D142 | **missing** | — |
| 18 | Tabs keyboard (arrows/Home/End in tab list) | D97 | partial | rune: Ctrl+Tab, Ctrl+PgUp/PgDn, drag reorder (rune:src/main.rs:879-884) |
| 19 | Diff summary of the selected project in title bar | spec/DiffSummary.md | done | K5 |
| 20 | Terminal: hc stream, scrollback, close on exit, re-attach, restore after restart | D79, D82, D199 | done | H3, H4; rune:src/hc/term.rs, rune:src/hc/restore.rs |
| 21 | Terminal text selection and copy (mouse drag, ⌘C) | MyGo terminal plugin (Go D81) | **missing** | rune grid has only press (focus) and wheel handlers (rune:src/main.rs:1231-1246); no selection, no copy |
| 22 | Clickable links in terminal (http/https/mailto only) | go:internal/app/links.go; Go D94 | **missing** | — |
| 23 | Terminal size: watch at hc's size until the first key, then size it (D91, D199) | D91, D199 | differs (kept) | rune resizes on select (rune:src/main.rs:392-393); owner keeps it (Q5). |
| 24 | Find in terminal (⌘F, title-bar Search) | D130, D143 | done | K7b; rune:src/find.rs |
| 25 | Split terminal | dropped in Go (D130) | n/a (rune extra) | K7c kept; rune:src/split.rs |
| 26 | Questions notch + card: questions, to-dos (Done), blocked agents; answer → `owner.json` + `Owner answered …` | D124, D135, D140; spec/QuestionsCard.md | done | H9; rune:src/notch.rs |
| 27 | Card: option impacts under options, refs + source in a collapsible section | D114, D125 | **missing** | rune `Question` has no `impacts`/`refs`/`source` (rune:src/hc/owner.rs:18-30) |
| 28 | Card: blocked worker shows its menu (parsed from screen) and sends a choice | D51; go:internal/prompt/prompt.go | partial | rune shows a button that opens the terminal (rune:src/notch.rs:255-259) |
| 29 | Orchestrator outline (ticks, hover card, new-count) | spec/Outline.md, spec/OutlineCard.md; go:internal/app/outline.go | **missing** | planned H10 (HC-PLAN §3) |
| 30 | Files panel: git tree, change counts, changed-only filter | spec/FileTree.md; D170 | done | K6; rune:src/files.rs, rune:src/main.rs:1059-1125 |
| 31 | Click a file → Review view (diff) at that file | go:internal/app/frame.go:266,406-414 | differs | rune opens it in the default app (rune:src/main.rs:1119). See Q7. |
| 32 | Review view: branches ahead of main, files or commits, code diff | D37; go:internal/app/review.go | **missing** | rune-kit has a diff view (`crates/rune-kit/src/diff/`) |
| 33 | Session panel: agent's Todos ("N of M") and Artifacts over the file tree | D195, D197; spec/SessionPanel.md | **missing** | — |
| 34 | Context meter "90k of 200k  45%" under the tree | D139, D144; spec/UsageMeter.md | **missing** | planned H13 |
| 35 | Claude and Codex plan meters | D139, D196 (moved to the sidebar foot) | **missing** | H11, in the sidebar foot (Q2 answered; Go D196). |
| 36 | Servers: "This Mac · local" row | D132, D154 | done | rune:src/sidebar.rs:281 |
| 37 | This Mac row shows "local · 12% CPU · 48% MEM" | D11, D126; spec/ServerRow.md | **missing** | — |
| 38 | hc problem line under the server row / reconnecting state | spec/ServerRow.md | partial | rune: `reconnecting…` over the terminal (rune:src/main.rs:1154-1156) and status in Settings |
| 39 | Settings: General / Agents / Updates tabs; versions contr + hc | D146, D186 | partial | rune: one page, Appearance + versions (rune:src/dialog.rs) |
| 40 | Settings → Agents → Roles (CLI/model/effort per role, writes `agents.json`) | D186; spec/SettingsAgents.md | **missing** | — |
| 41 | Settings → Agents → Flow (presets, plan/review/debugger/workers/merge) | D185, D187; spec/FlowSection.md | **missing** | — |
| 42 | Self-update from signed GitHub releases; update pill in title bar; dot on Settings | D115, D176, D198; spec/UpdatePill.md | **missing** | owner asked for it (Q8): R3a, R3b. Rune has a release workflow (v0.1.0) but no updater (rune:src/dialog.rs:6-7). |
| 43 | Toasts for action messages | D126 | partial | rune: one red error line in the sidebar foot (rune:src/sidebar.rs:283-285) |
| 44 | Narrow window: files hide < 1100 px, sidebar icon column < 800 px | D128, D141 | **missing** | — |
| 45 | Sidebar and panel widths dragged and kept | D111 | **missing** | — |
| 46 | ⌘B toggles the sidebar (View menu); files toggle is a title-bar button | D172 | differs | rune ⌘B toggles the files panel (rune:src/main.rs:854) |
| 47 | Native menu bar (contr, File, Edit, View, Window) with ⌘T, ⌘W, ⌘,, ⌘B | go:internal/app/menus.go, actions.go | **missing** | Rune has `native_menu` (rune checkout `crates/rune/src/native_menu/`) |
| 48 | Window opens at the screen's work area inset 20 | D167 | **missing** (minor) | rune opens 1280×800 (rune:src/main.rs:1404) |
| 49 | Board, worker panel, PR viewer | built but **hidden** (D122, D127, D147; `viewersEnabled = false` in go:internal/app/hidden.go) | n/a | Do not build (owner redesign pending). |

### 1.2 Agent-side tooling (not UI) — **required: owner chose Standalone (Q1)**

Agents depend on these, so contr-rune must ship them (lane D, §3.2). Go line counts are non-comment code lines
at `5d3ef7a`.

| What agents call | Go source (code lines) | Who calls it | contr-rune |
|---|---|---|---|
| `contr agents spawn` (`--name --profile --issue --worktree --project --role --review-pr --no-parent --json --task-file`, task words) | go:cmd/contr/agents.go (340), go:internal/agents/agents.go `Spawn` (575 whole file), worktree.go (117) | orchestrator playbook (`kit/commands/orchestrate.md`:13), app "+ new" (D57) | **missing** → D4a |
| `contr agents ls / send / interrupt / stop [--keep] / rm [--ended] / read [--lines] / prompt / flow / watch` | same files; flow.go (139) | playbook:9 (`contr agents flow` first), shell use | **missing** → D4a (flow, ls), D4b (rest; `watch` left out: nothing in the kit calls it) |
| `contr mcp`: tools spawn, message, interrupt, stop, read, prompt, list, clear_ended | go:cmd/contr/mcp.go (332), go:internal/mcp/server.go (160) | kit `.mcp.json` → every Claude session with the plugin | **missing** → D8 |
| `contr mcp`: tools ask_owner, decided, pin_for_owner, unpin_for_owner, owner_items | go:cmd/contr/mcp.go:330-357, go:internal/owner/owner.go (206) | orchestrator (playbook:23) | **missing** → D9 |
| `contr owner ask / answer / done` | go:cmd/contr/owner.go (147) | shell fallback of the MCP tools | **missing** → D3 |
| `contr hook stop` (worker: "ended its turn without a report. Last words: …" to `CONTR_COORDINATOR`; any session: `terminal.label` `contr.model`/`contr.effort`, D189; coordinator: ask for `COORDINATOR-STATE.md` past `CONTR_SAVE_AT`) | go:cmd/contr/hook.go (410), go:internal/agents/turn.go (157) | kit `hooks/hooks.json` Stop | **missing** → D5, D6 |
| `contr hook session-start` (after compaction: playbook, state file, live workers, open owner items, ≤ 9500 chars) | go:cmd/contr/hook.go:197-449 | kit SessionStart `compact` | **missing** → D6 |
| `contr hook post-tool`, `contr hook prompt` (session todos/artifacts file, `[you]` todos) | go:cmd/contr/hooksession.go (213), go:internal/session/session.go (124) | kit PostToolUse, UserPromptSubmit | **missing** → D7 |
| `contr statusline [-- <old command>]` (`usage/claude.json`, `usage/context/<id>.json`, chains the old status line with a 2 s timeout) | go:internal/usage/claude.go (227), context.go (179) | Claude Code `statusLine.command` | **missing** → D10 (= HC-PLAN H12) |
| Kit plugin: `agents/{builder,planner,reviewer}.md`, `commands/orchestrate.md`, `hooks/hooks.json`, `.mcp.json`, `.claude-plugin/plugin.json`; repo-root `.claude-plugin/marketplace.json` | go:kit/, go:.claude-plugin/ | `claude plugin install`; `agentArgv` adds `--plugin-dir <kit> --agent contr:<agent>` | **missing** → D11 |
| `hc send` (workers report to the coordinator) | hc itself | builder.md:26 | n/a: hc's own CLI, already a requirement |
| `contr terminal-check`, `--icons` legend | go:cmd/contr/terminalcheck.go | Go-only (MyGo plugin, legend window) | n/a |

### 1.3 Not gaps (deliberate differences, or not in current contr)

- **hc, not tmux.** Both Go contr and contr-rune use hc. Swift's tmux runtime (Swift `Sources/ContrKit/Tmux`, P2, P25) is gone from contr.
- **rune-kit neutral look**, not Go's tokens or glass (rune AGENTS.md "Design source"). Kit shapes are accepted.
- **Split terminal** is a rune extra (Go D130 dropped it).
- **Local PTY fallback** when hc is off (HC-PLAN O4) is a rune extra.
- **Swift-only, not in current contr:** kanban board of GitHub issues, slots/thermal control, `contr check` merge gate, inbox, presets/routing, daily summary, first-run window, dev panel, Developer Changelog window (Swift `docs/SPEC.md` §Workflow, §First run, §Resource control; DECISIONS P5-P33, D18, D22). Do not build.
- **Go dev/e2e harness** (MyGo automation, `scripts/see.sh`, spec sheet; D5, D148, D178): rune uses `--snap/--tree/--script` instead.

## 2. Designed only (not built in Go contr) — do not build ahead of the owner

| Item | Where designed | In rune's design? |
|---|---|---|
| Remote servers / devices and device filter, Servers `+` | Go issue #149 (open, priority low; "local only for now", #134); D132 | **Yes**: docs/design/main.png draws `linux-pc online`, `build-box offline`. Keep "this Mac" only. |
| Terminal lens (agent output as native UI) | Go issue #267 (open, low) | No. |
| Agent groups by state (`Blocked N`, `Working N`), line 2 `running <cmd>` / `no output · <age>` + Interrupt button | Go `docs/DESIGN.md` "(not built)" | No (rune follows the v0.3 spec rows). |
| Commits behind upstream `↓N` on project rows | Go `docs/DESIGN.md` "(not built)" | No. |
| Session panel "changed since last opened" dot | spec/SessionPanel.md "Not yet" | No. |
| Board, worker detail, PR viewer redesign | D122, D127, D147 (hidden, redesign pending) | No (rune main.html mentions "board" only in sample terminal text). |
| Settings → Agents: Codex and OpenCode CLIs | spec/SettingsAgents.md "come later" | No. |
| Swift SPEC v1 features (board columns, slots, checks, daily summary) | Swift `docs/SPEC.md` | No. |
| Open polish issues #431, #450, #416, #487 | Go issues | Go-specific (MyGo/tokens). Not applicable. |

## 3. Missing work as parts

Revised 2026-11-09 after the owner answers (end of file): Standalone (Q1), plan meters in the sidebar foot
(Q2), shared tab set (Q4), keep "resize on select" (Q5), update notices (R3).

Rules for every part are rune AGENTS.md and PARTS.md "Rules for every part": branch `part-<id>-<name>` off
`origin/main`, ≤ ~400 code lines, `--snap` as `docs/design/snaps/part<ID>.png` for a visible part, tree + script
checks as a test under `tests/`, `cargo build` with no warnings, full `flock … cargo test` once before the PR.
Fake core for every hc call (`CONTR_HC_FAKE`, `CONTR_HC_FAKE_LOG`). Sizes are code lines without tests.

### 3.1 UI parts (lanes A, B, C)

Ordered by value to the owner.

| Order | ID | Part | Done when | Size | Files |
|---|---|---|---|---|---|
| 0 | **R0** | Move views out of `main.rs` (prep, no behaviour change) — **in progress, scope unchanged** | `title_bar`/`trailing` → `src/titlebar.rs`; `terminal_panel`, `terminal_panel_body`, `reconnecting`, `find_total`, `split_row`, `pane`, `find_bar` → `src/terminal_view.rs`; `files_panel`/`file_tree` → `src/files.rs`. Tests pass unchanged; snaps identical. | ~0 net (≈450 moved) | `src/main.rs`, `src/titlebar.rs`, `src/terminal_view.rs`, `src/files.rs` |
| 1 | **T1** | Select and copy terminal text | Drag on the grid selects (alacritty `Selection`); double-click selects a word; selection drawn with a kit selection fill; ⌘C copies; typing to the shell or output that scrolls clears it. Works in both split panes and in scrollback. No mouse reports to programs (as today). | ~200 | new `src/select.rs`, `src/grid.rs`, `src/terminal_view.rs`, `src/main.rs` (1 field, ⌘C line) |
| 2 | **A1** | Agent row actions | Secondary press on `agent/<address>` opens a rune-kit `context_menu`: Interrupt (`terminal.interrupt`), Stop (`terminal.kill` + `terminal.rm`, D112), Open PR (`gh pr view --web` in the worktree; off when `gh pr view --json number` fails). Off the UI thread; a failure shows in the error line. | ~180 | `src/sidebar.rs`, `src/hc/watch.rs`, `src/hc/fake.rs` (append arm), `src/main.rs` (1 field, 1 `HcUpdate` arm) |
| 3 | **H11** | Plan meters in the **sidebar foot** (Q2, Go D196) | Sidebar foot row: "Claude ▬ 38%  Codex ▬ 12%" (label small faint, custom 44 px 4 px meter — kit `Progress` cannot be 4 px, rune-kit.md:47 — value mono), then the Settings gear. Plan usage does not follow the selection. Readers per HC-PLAN C8 (Claude file, Codex rollout), on a thread every 60 s while focused; "–" and an empty meter with no data. `src/usage.rs` also gets the pure context-file reader that H13 uses. | ~220 | new `src/usage.rs`, `src/sidebar.rs` (foot) |
| 4 | **A3** | New orchestrator (in-process spawn) | Projects `+` opens a menu: "New orchestrator…" (folder picker) and "Open folder…". Empty project row menu: "New orchestrator in <name>". It calls lane D's `agents::spawn` with profile `orchestrator`, role `orchestrator`, no parent, worktree = folder, project = folder name, name per K3 — off the UI thread; the row arrives with the hc `created` event; errors ("no orchestrator profile in <path>", hc off) show in the error line. **Needs D1, D2, D4a merged.** | ~120 | `src/sidebar.rs`, `src/main.rs` (1 `HcUpdate` arm) |
| 5 | **C5** | Projects persist; launch selection | Open projects are saved to `contr-rune/projects.json` (Q3) and come back at launch; `CONTR_PROJECTS` still overrides. Once per launch, select the first live orchestrator of the first project, else the first live worker (D173). Empty projects last (D174). | ~150 | new `src/projects.rs`, `src/main.rs` (`init`, `add_project`, `refresh_agents` hook) |
| 6 | **A2** | Blocked menu in the card | A blocked agent's card tab reads the screen (`terminal.read`, `lines:40`), parses it (`src/prompt.rs`, port of go:internal/prompt), shows detail, question and numbered choices; `1`–`9` or a click sends the choice with `terminal.keys`; "Open terminal" stays; unknown form → button only. Re-read on the agent's `status`/`turn` event. `src/prompt.rs` was built by D4b (`parse(command, kind, screen) -> Option<Prompt>` with `detail`, `question`, `choices`); A2 reuses it and adds what it needs (`text` choices, `Equal`). D8 uses it too. | ~280 | `src/prompt.rs`, `src/notch.rs, `src/hc/watch.rs`, `src/hc/fake.rs` (append arms) |
| 7 | **H13** | Context meter under the file tree | Go keeps Context under the tree (spec/UsageMeter.md "Under the file tree"; D197); only plan meters moved (D196). "90k of 200k  45%" of the selected agent, reader from `src/usage.rs` (H11). | ~80 | `src/files.rs` |
| 8 | **H10** | Orchestrator outline | As HC-PLAN H10 (C7 ticks, hover card, count). | ~250 | new `src/outline.rs`, `src/terminal_view.rs` (1 overlay line) |
| 9 | **R3a** | Update check, pill and toast (owner asked) | A check on a thread at launch and every 15 min on the wall clock while the window is visible, and on focus when the last good check is > 10 min old (D198): `GET https://api.github.com/repos/contursofran/contr-rune/releases/latest` with `/usr/bin/curl` (no new HTTP dependency); newer `tag_name` than `CARGO_PKG_VERSION` → "update ready". Title bar pill `Restart to update` left of the diff summary; one toast per version (rune-kit `toast`) with `Restart now` / `Later`; pill click → popover with version, from-version, "What's new" (≤ 6 list items of the release body). Settings shows "contr-rune 0.1.0 · up to date / 0.2.0 available" and `Check for updates`. `CONTR_UPDATE_URL` points tests at a file URL. No check in headless runs. | ~300 | new `src/update.rs`, `src/titlebar.rs` (pill), `src/dialog.rs` (1 row), `src/main.rs` (1 field, poll line) |
| 10 | **R3b** | Download, verify, install, restart | `Restart now`: download `contr-rune-macos-arm64.zip` and `.sha256` of that release (curl), verify (Q15), unzip with `ditto` to a temp dir, then a detached `/bin/sh` helper waits for this pid to exit, replaces the running `.app` (path from the executable, refuse when not inside a `.app`), clears quarantine, and `open`s it. Errors show in Settings, not as a toast (D176). Release workflow unchanged unless Q15 = signature. | ~200 | `src/update.rs`, `scripts/update-helper.sh` (embedded), maybe `.github/workflows/release.yml` (Q15) |
| 11 | **C4** | Session panel: todos and artifacts | Over the file tree: "Todos  N of M", "Artifacts  N" (title, age `now/38m/5h/2d`, click opens URL with `tx.open_url`), sections hidden when empty; head above the tree reads "Files". Reads K2 session file of the selected agent with lane D's `src/session_file.rs`, on a thread when the selection or the file's mtime changes (only while focused). **Needs D7 merged.** | ~200 | new `src/session_panel.rs`, `src/files.rs` (1 call) |
| 12 | **A6** | Question impacts, refs, source in the card | Card shows the nth impact under the nth option; a collapsed "Context" section with source and refs; `#N` / `issue N` / http refs open the browser, others dim. Fields come from D3. **Needs D3 merged.** | ~100 | `src/notch.rs` |
| 13 | **T2** | Links in the terminal | ⌘-click on an http/https/mailto URL opens it (`tx.open_url`); other schemes do nothing (Go D94). | ~100 | `src/select.rs`, `src/terminal_view.rs` |
| 14 | **T3** | Tab extras | Middle click closes a plain tab; secondary press: Close (off on agent tabs), Close others (D142). If `TabStrip` cannot report the press, stop and report (Rune limit). | ~100 | `src/titlebar.rs` |
| 15 | **T4** | One shared tab set (Q4 = yes, D190) | Tabs of the selected project: every Claude session (orchestrators, foreign Claude, plain tabs running Claude) in sidebar order, then the selected worker, then plain terminals; the set does not change with the selection; a plain tab keeps the last worker. Grey Claude mark for a non-orchestrating session (D191). | ~250 | new `src/tabs.rs`, `src/titlebar.rs`, `src/hc/model.rs`, `src/main.rs` (`select_agent`, `select_tab`) |
| 16 | **A4** | Issue titles and CI on agent rows | Off the UI thread on an agents change: `gh issue view <N> --json title`, `gh pr view --json number,state,statusCheckRollup` per worktree; line 1 = issue title (D174); line 2 adds `CI` (D145). Fake `gh` via `CONTR_GH`. | ~250 | new `src/github.rs`, `src/hc/model.rs`, `src/main.rs` (1 field, 1 poll arm) |
| 17 | **C6** | Projects modal | From "Open projects" in the `+` menu: filter, open list with drag reorder and `close` (hide), `recent` with `open`, `+ open folder…`, Done/esc. No clone. **Needs C5, A3 merged.** | ~300 | new `src/projects_modal.rs`, `src/projects.rs`, `src/sidebar.rs` (1 menu item) |
| 18 | **C7** | Settings tabs + Agents → Roles | `General` / `Agents` / `Updates` tabs (R3a's row moves to Updates); Roles: Orchestrator/Planner/Builder/Reviewer/Debugger with CLI/Model/Effort `Select`s and the argv line; "not set · suggested: claude --model opus --effort medium --permission-mode auto" for a missing orchestrator profile. Uses lane D's `src/agents_config.rs` (load/save). **Needs D2 merged.** | ~300 | `src/dialog.rs`, new `src/settings_agents.rs` |
| 19 | **C8** | Agents → Flow | spec/FlowSection.md; writes `flow` through `src/agents_config.rs`. | ~250 | new `src/flow_section.rs`, `src/dialog.rs` (1 call) |
| 20 | **A5** | This Mac CPU and memory | Server row meta "local · 12% CPU · 48% MEM" (D11 formula), sampled only while focused. New `libc` dependency (say why). | ~150 | new `src/host.rs`, `src/sidebar.rs`, `Cargo.toml` |

### 3.2 Standalone CLI parts (lane D)

One binary, `contr-rune`, with subcommands (recommended over a second binary: one build, one bundle, one
version, and the app already parses arguments before Rune starts per HC-PLAN H12). `main()` gets one line:
`if let Some(code) = cli::run(std::env::args_os()) { std::process::exit(code) }` before `rune::App::new`.
No Rune or window code runs for a subcommand. The CLI reuses `src/hc/conn.rs` and `src/hc/dial.rs` read-only
(the plain `Conn::call` already takes any method). File formats and paths stay Go-compatible (K2), so the
owner can move from Go contr without losing `owner.json`, `agents.json`, usage or session files.

| Order | ID | Part | Done when | Size | Files |
|---|---|---|---|---|---|
| D-0 | **D1** | CLI skeleton and paths | `contr-rune help`, unknown argument → message + exit 2; dispatch for `agents`, `owner`, `hook`, `mcp`, `statusline` (each "not built yet" exit 2 until its part); `src/paths.rs`: `state_dir()` (`CONTR_STATE_DIR`, else `~/Library/Application Support/contr`), `agents_config()` (`CONTR_AGENTS_CONFIG`, else `…/contr/agents.json`), `session_file(id)`, `context_file(id)`, `id` sanitising (K2). `hc` found as HC-PLAN §1.2. | ~120 | new `src/cli/mod.rs`, new `src/paths.rs`, `src/main.rs` (1 line) |
| D-1 | **D2** | `agents.json` config | Load over defaults (Go `DefaultConfig`: builder sonnet/medium, planner opus/low, reviewer opus/low, debugger opus/xhigh, all `--permission-mode auto`; flow defaults), a file profile replaces the default of that name; `model_of`/`effort_of`; `Flow` validate + `lines()` text (as go:internal/agents/flow.go); save keeps unknown keys (`serde_json::Value` round trip, tmp + rename). Unit tests only. | ~250 | new `src/agents_config.rs` |
| D-2 | **D4a** | `agents spawn`, `ls`, `flow` | Spawn per K3 (labels, kit agent argv, setup command + `CONTR_MAIN`, `CONTR_COORDINATOR` + `parent` from `HC_TERMINAL_ID` unless `--no-parent`, task words or `--task-file`, `--json`). `ls [--all] [--json]` lists `contr=agent` terminals sorted by name; `flow` prints the flow text. A pure `spawn_params(spec, config, env) -> serde_json::Value` is the function A3 also calls. Test: fake core + log has `terminal.run` with K3 params. | ~350 | new `src/cli/agents.rs`, new `src/agents.rs` (spawn core shared with A3) |
| D-3 | **D4b** | `agents send / interrupt / stop / rm / read / prompt`, `--review-pr` | `send` refuses blank text (D200) and reports `terminal.send` refusals; `stop` = kill + rm (`--keep` kills only); `rm --ended` removes finished workers whose `parent` is `HC_TERMINAL_ID`; `read --lines N`; `prompt` = `terminal.read` + `src/prompt.rs` (needs A2 merged) or "no prompt"; `--review-pr N` makes `<main>-wt/review-<N>` detached at the PR head (`gh pr view N --json headRefOid`, `git worktree add --detach`). `HC_TERMINAL_ID` sent as `caller` on kill/read/interrupt (D83). | ~330 | `src/agents.rs`, `src/cli/agents.rs` |
| D-4 | **D3** | Owner store writes + `owner ask / answer / done` | `Question` gets `impacts`, `refs`, `source` (optional; D114); `ask` appends with next id (`D<n>`), project from `CONTR_PROJECT` else cwd folder, coordinator = `HC_TERMINAL_ID`; `pin`/`unpin` to-dos (`T<n>`, kind `do|know`); `decided` closes by owner/coordinator; lock + tmp + rename as C7, unknown fields kept. `answer`/`done` = what the card does (send fails → saved, exit 1). | ~250 | `src/hc/owner.rs` (lane D owns it from here), new `src/cli/owner.rs` |
| D-5 | **D8** | MCP server + agent tools | `contr-rune mcp`: JSON-RPC 2.0 on stdio, `initialize`, `tools/list`, `tools/call`, `ping`; tools `spawn`, `message`, `interrupt`, `stop` (`keep`), `read` (`lines`), `prompt`, `list`, `clear_ended` with Go's descriptions and argument schemas (go:cmd/contr/mcp.go:197-260); unknown arguments refused (D200). Profiles file read on each spawn. | ~380 | new `src/cli/mcp.rs`, new `src/cli/mcp_tools.rs` |
| D-6 | **D9** | MCP owner tools | `ask_owner` (title, from, question, options, recommended, impacts, refs, source), `decided`, `pin_for_owner`, `unpin_for_owner`, `owner_items` (`all`), texts as go:cmd/contr/mcp.go:330-357. | ~200 | `src/cli/mcp_tools.rs` |
| D-7 | **D5** | `hook stop` (worker report + model labels) | Reads one JSON value from stdin with a timeout (Swift P21 lesson). Worker (`CONTR_COORDINATOR` set): last turn from `transcript_path` (port of go:internal/agents/turn.go `LastTurn`); no `hc send`/`message` call in the turn → `terminal.send` to the coordinator: `<hc from="<address> <name>">ended its turn without a report. Last words: <one line ≤ 160>` (exact text from go:cmd/contr/hook.go `missedReport`). Any hc session: `terminal.label` `contr.model`/`contr.effort` (D189). Never exits 2. | ~300 | new `src/cli/hook.rs`, new `src/cli/turn.rs`, `src/hc/fake.rs` (append `terminal.label`) |
| D-8 | **D11** | Kit, marketplace, install | Port `kit/` (agents, `commands/orchestrate.md`, `hooks/hooks.json`, `.mcp.json`, `plugin.json`) with every `contr …` command renamed to `contr-rune …` (Q16), repo-root `.claude-plugin/marketplace.json`; `hooks.json` lists only hooks built so far (Stop now; D6/D7 add theirs). App bundle ships `kit/` in `Contents/Resources/kit` (`scripts/bundle-app.sh`); `scripts/install-cli.sh` links `~/.local/bin/contr-rune` to the bundle binary. Owner check: `claude plugin marketplace add <repo>`, `claude plugin install contr@contr-rune`, `/contr:orchestrate <goal>` spawns a worker that reports back. | ~60 code + docs | new `kit/**`, new `.claude-plugin/marketplace.json`, `scripts/bundle-app.sh`, new `scripts/install-cli.sh` |
| D-9 | **D6** | Coordinator compaction + `hook session-start` | Coordinator stop past `CONTR_SAVE_AT` (default 220000; compact at `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, default 250000) keeps the turn once and asks for `COORDINATOR-STATE.md` (marker per session); `session-start` (source `compact`) prints playbook + state-file note + live workers + open owner items, ≤ 9500 chars. Adds the SessionStart entry to `kit/hooks/hooks.json`. | ~300 | `src/cli/hook.rs`, new `src/cli/restore.rs`, `kit/hooks/hooks.json` |
| D-10 | **D7** | `hook post-tool`, `hook prompt`, session file | `src/session_file.rs` (K2 types, read, write tmp + rename, ≤ 20 artifacts, `[you] ` prefix); PostToolUse: `TodoWrite` replaces, `TaskCreate`/`TaskUpdate` by id, artifact calls move the slug to the top; UserPromptSubmit asks Claude to add each owner request as a `[you] ` todo, skips `<hc from=` prompts and workers. Always exit 0. Adds both entries to `hooks.json`. | ~280 | new `src/session_file.rs`, new `src/cli/hook_session.rs`, `kit/hooks/hooks.json` |
| D-11 | **D10** | `statusline` (= HC-PLAN H12) | As HC-PLAN H12 / C8 writers, plus `contr-rune statusline -- <old command…>` chains the old status line with a 2 s timeout and prints its output (go:internal/usage/claude.go:157-191); input > 1 MiB → nothing saved, old command not run. Owner check: `statusLine.command` = `contr-rune statusline -- ~/.claude/statusline.sh`; the sidebar Claude meter moves. | ~300 | new `src/cli/statusline.rs` (writers; readers stay in `src/usage.rs`) |

**Before A3:** D1 → D2 → D4a must be merged (A3 calls `src/agents.rs` spawn and reads `agents.json`). For the
new orchestrator to be *useful* (spawn workers, report, ask the owner), D4b, D8, D9, D5 and D11 must also be
merged and the kit installed; A3 can merge before them, but the owner check of A3 ("start an orchestrator and
give it a goal") waits for D11.

Later, only on an owner decision (not scheduled): Review view (Q7), glass + transparency sliders (Q9), narrow
window + draggable/kept widths, native menu bar, window opens at work area, toasts instead of the error line,
remote servers (#149).

## 4. Parallel schedule (4 lanes)

### How `src/main.rs` stays conflict-light

1. **R0 first** (in progress). After it, `main.rs` holds `App`, `init`, the `App` methods, `setup`, `poll`,
   `on_key`, `view` and `main` (scenes).
2. **Hook protocol:** the feature lives in its own module; `main.rs` gets at most one `App` field (the
   module's state struct), one `init` line, one `poll` line, one `on_key` line, one `HcUpdate` arm, one scene.
   Target ≤ 15 changed lines in `main.rs` per part. Lane D adds exactly one line to `main()` (D1) and nothing
   else.
3. Each lane **owns** files; another lane does not edit them. Shared, append-only exceptions:
   `src/main.rs` (hooks above) and `src/hc/fake.rs` (new method arms only: A1 `terminal.interrupt`,
   A2 `terminal.read`/`terminal.keys`, D5 `terminal.label`). On rebase keep both sides.
4. Rebase on `origin/main` right before the PR; run only the part's test file while working, the full suite
   once under `flock /tmp/contr-rune-tests.lock`.

| Lane | Owns | Parts, in order |
|---|---|---|
| **A · terminal, title bar, updates** | `src/grid.rs`, `src/terminal_view.rs`, `src/titlebar.rs`, `src/find.rs`, `src/split.rs`, `src/keys.rs`, new `src/select.rs`, `src/outline.rs`, `src/tabs.rs`, `src/update.rs` | R0 → T1 → H10 → R3a → R3b → T2 → T3 → T4 |
| **B · sidebar, agents view, hc client, meters** | `src/sidebar.rs`, `src/notch.rs`, `src/hc/{watch,model,term,restore,…}.rs` (not `owner.rs`), new `src/usage.rs`, `src/prompt.rs`, `src/github.rs`, `src/host.rs` | A1 → H11 → A3 → A2 → A6 → A4 → A5 |
| **C · right panel, settings, projects** | `src/files.rs`, `src/dialog.rs`, `src/settings.rs`, new `src/projects.rs`, `src/projects_modal.rs`, `src/session_panel.rs`, `src/settings_agents.rs`, `src/flow_section.rs` | C5 → H13 → C7 → C8 → C4 → C6 |
| **D · standalone CLI and kit** | new `src/cli/**`, `src/paths.rs`, `src/agents_config.rs`, `src/agents.rs`, `src/session_file.rs`, `src/hc/owner.rs`, `kit/**`, `.claude-plugin/**`, `scripts/install-cli.sh`, `scripts/bundle-app.sh` | D1 → D2 → D4a → D4b → D3 → D8 → D9 → D5 → D11 → D6 → D7 → D10 |

Lane D is file-disjoint from the UI lanes apart from the one `main()` line, `src/hc/fake.rs` arms and
`src/hc/owner.rs` (moved to lane D; lane B's `notch.rs` only reads it). Cross-lane needs are all "needs merged"
rows below (A3 ← D4a, A6 ← D3, C4 ← D7, C7 ← D2, D4b ← A2).

**Start order now:**
- Lane A: R0 (running) → T1 stacked on R0's branch.
- Lane B: A1 now (independent of R0).
- Lane C: C5 now (independent); H13 after H11 merges.
- Lane D: D1 now, then D2 stacked on D1.
Then lane B does H11 while D4a is built, so A3 can start as soon as D4a merges.

## Contract

Additions to HC-PLAN's Contract (C1-C9 stay the source for what they cover).

### K1. hc methods added (JSON-RPC, as C1; schema `docs/design/hc/hc.openrpc.json`)

| Method | Params rune sends | Result rune reads | Parts |
|---|---|---|---|
| `terminal.interrupt` | `{"terminal":"t17"}` (+ `"caller":HC_TERMINAL_ID` from the CLI) | `{outcome, key}`; `outcome` ∈ `stopped`, `exited`, `still_working`, `unknown` | A1, D4b, D8 |
| `terminal.kill` then `terminal.rm` | `{"terminal":"t17"}` (+ `caller`) | as C1 | A1, D4b, D8 |
| `terminal.read` | `{"terminal":"t17","lines":40}` (+ `caller`) | `{text, complete, how, why?}` | A2, D4b, D8 |
| `terminal.keys` | `{"terminal":"t17","keys":["2"]}` (the choice digit; add `"enter"` only if go:internal/app/questions.go does — builder checks) | ignored | A2 |
| `terminal.send` | `{"terminal":<coordinator>,"text":"…\n"}` | `{turn, started?, status?, wait?}`; error kind `blocked` | D4b, D5, D8, D3 |
| `terminal.label` | `{"terminal":HC_TERMINAL_ID,"labels":{"contr.model":"opus","contr.effort":"high"}}` | ignored | D5 |
| `terminal.run` | K3 spawn params | `RunResult` | D4a, A3, D8 |
| `terminal.list` | `{"all":<bool>,"labels":{"contr":"agent"}}` | as C1 | D4a, D4b, D8, D6 |

Fake core (C9) additions, append-only arms: `terminal.interrupt` → `{"outcome":"stopped","key":"esc"}`;
`terminal.read` → `{"text":<fixture screens[t]>,"complete":true,"how":"screen"}`; `terminal.keys`,
`terminal.label` → `{}`. Every one logs `{"method":…,"params":…}`. Errors per fixture `errors`. The CLI uses the
fake the same way the app does (`CONTR_HC_FAKE` → in-process fake; the CLI is never headless-gated, so tests
always set it).

### K2. Files (Go-compatible paths and shapes)

| File | Path | Shape | Writer / reader |
|---|---|---|---|
| Profiles | `CONTR_AGENTS_CONFIG`, else `~/Library/Application Support/contr/agents.json` | `{"kit":DIR,"profiles":{NAME:{"argv":[...],"agent":KIT_AGENT}},"setup":{MAIN_CHECKOUT:SHELL_CMD},"flow":{"plan":"never|when-needed|always","review":"none|code|every","debugger_after":N,"workers":N,"merge":"ask|auto"}}`; unknown keys kept | D2 (`src/agents_config.rs`) owns load/save; C7, C8 write through it; D4a, A3 read |
| Owner | C7 (`CONTR_OWNER_FILE`, else `~/Library/Application Support/contr/owner.json`) | C7 + `impacts:[string]`, `refs:[string]`, `source:string`, all optional (D114) | D3, D9 write; card (H9, A6) reads/answers |
| Session | `<state>/session/<id>.json`; `<state>` = `CONTR_STATE_DIR`, else `~/Library/Application Support/contr`; `<id>` = `HC_TERMINAL_ID`, chars outside `[A-Za-z0-9_-]` → `_` | `{"schema":1,"updated_at":u64,"todos":[{"id"?,"text","active","status":"pending|in_progress|completed","you":bool}],"artifacts":[{"slug","title","url","updated_at":u64}]}`, ≤ 20 artifacts | D7 writes (`src/session_file.rs`); C4 reads |
| Usage | C8 | unchanged | D10 writes; H11, H13 read (`src/usage.rs`) |
| Projects | `CONTR_PROJECTS_FILE`, else `~/Library/Application Support/contr-rune/projects.json` (Q3: rune-only) | `{"version":1,"open":[path…]}`; C6 adds `"hidden":[path…]`, `"recent":[{"path","at":RFC3339}]` (≤ 10) | C5, C6 |
| Coordinator markers | `<state>/hooks/asked-<session_id>` (empty file) | — | D6 |

### K3. Spawn (D4a; same function for A3 and the MCP `spawn` tool)

`terminal.run` params:
`{"argv":<argv>,"name":<name>,"labels":{"contr":"agent","role":<role or profile>,"profile":<profile>,"worktree":<abs dir>,"project"?:<p>,"issue"?:<i>},"wait":false,"outlive_parent":true,"cwd"?:<worktree>,"env"?:{"CONTR_COORDINATOR"?:<parent>,"CONTR_MAIN"?:<main checkout>},"parent":<parent> | "no_parent":true}`
- argv = profile argv; if `argv[0]` basename is `claude` and `<kit>/agents/<agent>.md` exists (agent = profile
  `agent`, else profile name; kit = `CONTR_KIT`, else config `kit`, else the bundled `Contents/Resources/kit`):
  insert `--plugin-dir <kit> --agent contr:<agent>` after `argv[0]`. Task text appended as the last argument.
- setup: when `setup` has the worktree's main checkout, argv becomes `["/bin/sh","-c",<wrapper>,"contr-setup",<setup>,argv…]`
  (Go's `withSetup`; the wrapper runs `/bin/sh -c "$1"`, reports a failed setup, then `exec "$@"`; see `SETUP_WRAPPER` in `src/agents.rs`).
- parent = `HC_TERMINAL_ID` unless `--no-parent`; A3 always no parent.
- A3 name: `orch-` + folder lowercased, `[^a-z0-9_-]+` → `-`, trimmed of `-`, cut so the total with suffix
  `-MMSS` is ≤ 32 (go:cmd/contr/neworch.go `orchestratorName`); role `orchestrator`; project = folder name.
- No `orchestrator` profile → error `no "orchestrator" profile in <path>`.

### K4. CLI surface (`contr-rune <cmd>`; exit 0 ok, 1 error, 2 usage)

`agents ls [--all] [--json]` · `agents spawn …` (flags as go:cmd/contr/agents.go usage) · `agents send <name> <text…>` ·
`agents interrupt <name>` · `agents stop <name> [--keep]` · `agents rm <name> | --ended` · `agents read <name> [--lines N]` ·
`agents prompt <name>` · `agents flow` · `owner ask [--title --from --option… --impact… --recommended N --ref… --source] <q…>` ·
`owner answer <id> <answer…>` · `owner done <id>` · `mcp` · `hook stop|session-start|post-tool|prompt` ·
`statusline [-- <cmd…>]` · `help`. Environment: `CONTR_HC`, `CONTR_HC_FAKE`, `CONTR_HC_FAKE_LOG`, `CONTR_KIT`,
`CONTR_AGENTS_CONFIG`, `CONTR_OWNER_FILE`, `CONTR_STATE_DIR`, `CONTR_PROJECT`, `CONTR_COORDINATOR`,
`CONTR_SAVE_AT`, `HC_TERMINAL_ID`.
MCP tool names, descriptions and argument names: exactly Go's (go:cmd/contr/mcp.go:197-357), so the kit
playbook text stays valid.

### K5. Commands the app runs

| Command | cwd | Part | Test seam |
|---|---|---|---|
| `gh pr view --json number`, `gh pr view --web` | agent worktree | A1 | `CONTR_GH` |
| `gh issue view <N> --json title`; `gh pr view --json number,state,statusCheckRollup` | agent worktree | A4 | `CONTR_GH` |
| `gh pr view <N> --json headRefOid`; `git worktree add --detach` | repo | D4b | `CONTR_GH` |
| `/usr/bin/curl -fsSL <release API or asset URL>` | — | R3a, R3b | `CONTR_UPDATE_URL` (a `file://` JSON) |
| `/usr/bin/ditto -x -k`, helper `/bin/sh`, `/usr/bin/xattr -dr com.apple.quarantine`, `/usr/bin/open` | temp | R3b | `CONTR_UPDATE_DRY=1` stops before the swap |

Headless runs never run real `gh` or `curl`.

### K6. Element keys (new)

- T1: `terminal/selection` (label = selected text).
- A1: `agent/<address>/menu`, `agent/<address>/menu-item0|1|2` (Interrupt, Stop, Open PR; rune-kit fixes the part names).
- A3: `add-project/menu`, `add-project/menu/new-orchestrator`, `add-project/menu/open-folder`, `project-<i>/menu/new-orchestrator`.
- H11: `meter/claude`, `meter/codex` (in the sidebar foot); H13: `meter/context`.
- A2: `qcard/prompt/<address>`, `qcard/choice/<address>/<n>` (n from 1).
- A6: `qcard/impact/<ID>/<i>`, `qcard/context/<ID>`, `qcard/ref/<ID>/<i>`.
- R3a/R3b: `update-pill`, `update-panel`, `update-panel/restart`, `update-panel/later`, `update-toast`, `version/contr-rune/value`, `settings-check-updates`, `update/error`.
- C4: `session`, `session/todos`, `session/todos/count`, `session/todo/<i>`, `session/artifacts`, `session/artifact/<i>`.
- T3: `tab/<i>/menu`, `tab/<i>/menu/close`, `tab/<i>/menu/close-others`.
- C6: `projects-modal`, `projects-modal/filter`, `projects-modal/open/<i>`, `projects-modal/close/<i>`, `projects-modal/recent/<i>`, `projects-modal/done`.
- C7/C8: `settings-tabs`, `role/<profile>/cli`, `role/<profile>/model`, `role/<profile>/effort`, `role/<profile>/argv`, `flow/preset`, `flow/plan`, `flow/review`, `flow/debugger-after`, `flow/workers`, `flow/merge`.
- A5: `server-this-mac` meta text.

## Dependency table

| Slice | Dependency |
|---|---|
| R0 | independent (running now) |
| T1 | stacks on R0's branch |
| H10 | needs R0 merged; stacks on T1's branch if T1 is not merged (`terminal_view.rs`) |
| R3a | needs R0 merged (`titlebar.rs`) |
| R3b | stacks on R3a's branch |
| T2 | stacks on T1's branch |
| T3 | needs R0 merged; after R3a merged (both edit `titlebar.rs`) |
| T4 | needs T3 and A4 merged (`src/hc/model.rs`) |
| A1 | independent |
| H11 | stacks on A1's branch (`src/sidebar.rs`) |
| A3 | needs D4a merged (and D1, D2 under it); stacks on H11's branch (`src/sidebar.rs`) |
| A2 | needs A1 merged (`src/hc/watch.rs`, `src/hc/fake.rs`) |
| A6 | needs D3 merged; stacks on A2's branch (`src/notch.rs`) |
| A4 | needs A1 merged (`src/hc/model.rs`) |
| A5 | stacks on A3's branch (`src/sidebar.rs`) |
| C5 | independent |
| H13 | needs R0 and H11 merged (`src/files.rs`, `src/usage.rs`) |
| C7 | needs D2 merged; needs R3a merged (moves its Settings row into the Updates tab) |
| C8 | stacks on C7's branch |
| C4 | needs D7 and H13 merged (`src/files.rs`) |
| C6 | needs C5 and A3 merged |
| D1 | independent |
| D2 | stacks on D1's branch |
| D4a | needs D1 merged; stacks on D2's branch |
| D4b | needs A2 merged (`src/prompt.rs`); stacks on D4a's branch |
| D3 | needs D1 merged (independent of D2/D4) |
| D8 | needs D4b merged |
| D9 | needs D3 merged; stacks on D8's branch |
| D5 | needs D4a merged (spawn env `CONTR_COORDINATOR`); independent of D8 |
| D11 | needs D8, D9, D5 merged |
| D6 | needs D11 merged (`kit/hooks/hooks.json`) |
| D7 | stacks on D6's branch (`hooks.json`, `src/cli/hook*.rs`) |
| D10 | needs D1 merged; independent of the rest of lane D |

Longest "needs merged" chain: D1 → D4a → (A2) → D4b → D8 → D11 → D6 → D7 → C4. D4b's wait on A2 can be
cut by moving `src/prompt.rs` into D4b if A2 is late (then A2 needs D4b merged instead).

## 5. Open questions

### Answered (2026-11-09, see "Owner answers")

Q0 Go contr · Q1 Standalone · Q2 sidebar foot · Q4 adopt D190 · Q5 keep resize on select · Q8 build R3 ·
coordinator defaults for Q3, Q6, Q7, Q9, Q10, Q11, Q12, Q13, Q14 (as recommended).

### New (owner)

| # | Question | Recommendation |
|---|---|---|
| Q15 | Update integrity: today's release has only a `.sha256` next to the zip in the same release (rune `.github/workflows/release.yml`:37-49), so a tampered release passes. Add an ed25519 signature (Go uses a signed key, D115)? | **Yes:** sign the zip in the release job with a key kept in a GitHub secret; the app embeds the public key and verifies with `ed25519-dalek` (one new dependency). Until then R3b can ship with sha256 only and say so in Settings. |
| Q16 | Command and plugin names: the binary is `contr-rune` (kit commands become `contr-rune agents …`); keep the plugin name `contr` (so `/contr:orchestrate`, `contr:builder` and Go's `contr.model` labels stay the same)? | **Yes:** binary `contr-rune`, plugin `contr`. Uninstall Go's plugin first (two plugins named `contr` clash). A `contr` symlink can come later, when Go contr is gone. |
| Q17 | Codex and OpenCode workers: Go's CLI is Claude-only for kit agents (`agentArgv` only for `claude`). Keep it so? | **Yes** (parity); other CLIs later with Settings → Agents "come later". |
| Q18 | `agents watch` (prints JSON on each change): nothing in the kit calls it. Port it? | **No**; add it only when something uses it. |

## Owner answers (2026-11-09)

- Q0: Go contr (GitHub main, v0.2.48) is the target ("contr mygo").
- Q1: **Standalone.** contr-rune must work without Go contr installed. It replaces Go contr.
- Q4: Adopt Go's shared tab set (D190).
- Q5: rune replaces Go: keep rune's "resize on select".
- Q2: Plan meters in the **sidebar foot** (Go D196), not the files panel.
- Coordinator defaults (planner recommendation): Q3 rune-only projects file; Q6 keep ⌘B = files panel; Q7 Review view later;
  Q9 glass later; Q10 build Roles (C7) then Flow (C8); Q11 R0 allowed as an exception; Q12 ⌘C only, drag + double-click word;
  Q13 do not build board/worker panel/PR viewer; Q14 "this Mac" only.
- Q8 override: the owner asked for in-app update notices, so R3 (update check, pill, toast, install and restart) stays.

## Owner answers, round 2 (2026-11-09, night)

- Overnight flow: merge each PR on green CI plus the headless checks; the owner tests merged parts in a real window
  in the morning, and fixes become new PRs.
- Q15: **ed25519 signature.** The release job signs the zip with a private key stored as a GitHub secret; the app
  embeds the public key and rejects a download that does not verify.
- Q16: **Yes, all of it.** Binary `contr-rune`, plugin name stays `contr`. When D11 is done, uninstall Go contr's
  plugin on this Mac and install rune's kit.
- Q17 (default): kit agents stay Claude-only, as in Go. Q18 (default): do not port `agents watch`.
