# HC-PLAN: agent data and terminals from hc (herdr-core) in contr-rune

Status: plan, 2026-11-08. Read-only research; builders commit this file as `docs/design/HC-PLAN.md` and the two
schema files as `docs/design/hc/hc.openrpc.json` and `docs/design/hc/API.md` (copies of herdr-core
`schema/hc.openrpc.json` and `schema/README.md`, v0.1.19, Apache-2.0).

Source names used below:
- **[hcR]** herdr-core `README.md`; **[hcA]** `schema/README.md` (= `docs/design/hc/API.md`); **[hcS]**
  `schema/hc.openrpc.json` (= `docs/design/hc/hc.openrpc.json`); all `github.com/herdrdev/herdr-core@master`, v0.1.19.
- **[skill]** output of `hc --skill` (hc 0.1.19 on this Mac).
- **[go]** `github.com/contursofran/contr@5d3ef7a` (main, Go): the existing hc client. No local checkout; read with `gh api`.
- https://herdr.dev/core/ only gives the install line, `hc update`, `hc schema` and OSC 7501; the repo docs are the API source.

## Decisions (2026-11-08)

| # | Decision | By |
|---|---|---|
| O1 | Runtime is **hc** (herdr-core). Not the herdr app, not the herdr-agents board. | owner |
| O2 | Questions and to-dos: **share Go contr's `owner.json`** (same path, same lock, same answer format, same `Owner answered D7: …` send). Go contr's MCP tools `ask_owner` / `pin_for_owner` stay the writer. The reader sits behind a small trait (`OwnerSource`) so another source can be added later. | coordinator (owner did not choose) |
| O3 | Status reporter: not decided. Findings below (§1.3); a probe part (H7) checks it with the owner; a hook part is only added if the probe fails. | open |
| O4 | **Tabs become hc terminals** (as Go contr D82). Local PTY code stays only as the fallback when hc is off or not installed (`CONTR_HC=off`), and for headless tests that do not use the fake core. | owner |
| O5 | Reading `~/.codex/sessions` read-only is allowed. Claude/Context meters: a later `contr-rune statusline` subcommand writes the same usage files as Go contr, so Claude Code's `statusLine` can point to it. | owner |
| O6 | contr-rune may start the core through `hc connect`. | owner |

## 1. What hc is

### 1.1 Program and install
- herdr-core: "runs programs and coding agents in terminals that stay alive after you leave… Core draws nothing"
  ([hcR] 1-6). Rust, Apache-2.0 ([hcS] `info.license`).
- Install: `curl -fsSL https://releases.herdr.dev/core/install.sh | sh` ([hcR] 13). It installs
  `~/.local/bin/herdr-core` and `hc` beside it (`distribution/install.sh`:6,14); on macOS the core is the app
  "Herdr Core" (CHANGELOG 0.1.18, line 23). Update: `hc update` ([hcR] 25).
- **On this Mac now:** hc 0.1.19 is installed: `~/.local/bin/hc` and `~/.local/bin/herdr-core` →
  `~/.local/share/herdr-core/current/Contents/MacOS/herdr-core`. `hc server status` reports core
  `francos-macbook-air`, 0 terminals. No templates (`~/.config/herdr-core/templates` does not exist).
- Not hc: `~/.local/bin/herdr` (0.9.3) is the herdr app with its own API (`herdr api`, protocol 22).
- Sessions: `HC_SESSION=<name>` selects a separate core; socket `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`, else
  the state dir `~/.local/state/herdr-core/<session>/`; `HC_SOCKET` overrides (herdr-core `docs/security.md`:35-77).
  Tests that need a real core use their own `HC_SESSION` and scratch XDG dirs ([skill] "Never `hc server stop`…").

### 1.2 Transport
- **`hc connect`** as a child process: JSON-RPC 2.0, one message per line on stdin/stdout ([hcA] 21-43). It finds the
  socket and starts the core when needed ([go] DECISIONS D28), which O6 allows.
- **One connection for calls, one more per `events.subscribe`**: a subscribed plain connection carries only
  notifications ([hcA] 113-117; [go] `internal/hc/client.go`:5-7).
- **Framed mode** for terminal streams: `hello {"framed":true}` switches the connection after the reply
  ([hcA] 42-43). Frame = `[channel u32 LE][kind u8][len u32 LE][payload]`, payload ≤ 16 MiB ([hcA] 47-58).
  Channel 0 carries JSON-RPC both ways, so one framed connection can call and stream.
- Errors: `{"code":-32000,"message":…,"data":{"kind":"not_found","hint":…}}` ([hcA] 37-40). Branch on `kind`.
- Finding hc: `CONTR_HC`, else `hc` on PATH, else `~/.local/bin/hc`, `~/.local/bin/herdr-core`,
  `/opt/homebrew/bin/hc`, `/usr/local/bin/hc` (an app opened from Finder has a short PATH; [go] client.go:81-101).

### 1.3 Agent status (O3 findings)
- hc stores only what the program reports with OSC 7501 (Program Status Protocol): `idle`, `working`, `done`,
  `blocked`, `error`; "plain programs report none. Do not guess status from screen text" ([hcA] 214-219; [skill]
  "Status, recovery, and cleanup"). hc has **no command to set status** from outside ([go] D90 notes the same).
- hc ships **no built-in templates** and no hooks for Claude Code or Codex. Templates are user files in
  `~/.config/herdr-core/templates/<name>.md`, and "do not install agents or configure their status reporting"
  (herdr-core `docs/templates.md`:3-4). The template `ready` default waits for a 7501 report.
- Evidence that Claude Code reports 7501 by itself: the herdr app maps an agent's 7501 `app` value `claude-code` to
  `claude` (`github.com/herdrdev/herdr-app` `src/programs.rs`:195-196), and its agent guide defines "Agents are
  terminals whose program reports a status (OSC 7501)" (`defaults/AGENTS.md`:153). Not verified on this Mac.
- Codex: "Codex must run with `--no-daemon` (including templates), or its commands execute outside hc" ([skill] last
  lines). Whether Codex reports 7501 is not documented.
- Go contr's kit hooks (`kit/hooks/hooks.json`) do **not** report status; they only send reports and labels.
- Therefore: H7 is a probe with an owner check. A hook part (H7b) is added only if the probe shows no status.

### 1.4 Where Go contr differs from the hc docs
- `terminal.send` result: [go] `internal/agents/agents.go`:583-592 reads `{sent, reason}`; hc 0.1.19 returns
  `{turn, started?, status?, wait?}` ([hcS] `SendResult`, line 909) and refuses with error kind `blocked`.
- Events: [go] `internal/agents/watch.go`:16-17 says event data is undocumented and calls `terminal.get` per event;
  [hcA] 127-140 now documents each event's data and how to apply it. contr-rune applies the data in place.
- Plain tabs: [go] D82 runs `$SHELL -l`; hc now starts the machine's login shell when `argv` is empty
  ([hcS] `terminal.run` param `argv`; [skill] "`hc run` without a command starts this machine's own shell… login
  shell" on macOS).

## Contract

The single source of truth for the client, the fake core, the views and the tests.

### C1. hc methods (JSON-RPC over `hc connect`)

Request: `{"jsonrpc":"2.0","id":<u64>,"method":M,"params":{…}}`. Reply: `{"id":…,"result":…}` or `{"id":…,"error":{code,message,data{kind,hint}}}`.

| Method | Params contr-rune sends | Result | Ref |
|---|---|---|---|
| `hello` | `{"generation":1}`; `{"generation":1,"framed":true}` on stream connections | `{generation, version, methods[], features[], framed, core{id,name}}`. Refuse unless `generation == 1` | [hcS] 306, 1450 |
| `server.status` | `{}` | `{core{id,name}, version, pid, uptime_ms, live, finished, holders[]…}` | [hcS] 987, 1529 |
| `terminal.list` | `{"all":false,"labels":{…}}` | `{terminals:[Terminal], hidden_finished, machines}` | [hcS] 1878 |
| `terminal.get` | `{"terminal":"t7"}` | `Terminal`; error `not_found` | [hcS] 1930 |
| `terminal.run` | new tab: `{"argv":[],"cwd":<project path>,"name":"term-<MMSS><n>","labels":{"contr":"agent","role":"terminal","project":<name>,"worktree":<path>},"no_parent":true,"outlive_parent":true,"wait":false,"size":{"cols":C,"rows":R},"env":{"TERM":"xterm-256color","COLORTERM":"truecolor"}}` | `RunResult` (a Terminal record + `output`, `wait`); errors `name_taken`, `spawn_failed`, `unavailable`, `invalid_params`, `config` | [hcS] 1625, 660 |
| `terminal.attach` | `{"terminal":"t7","cols":C,"rows":R,"history":1000}`; after a lost line add `"resumed":true` | `{channel, terminal, automatic_input?, resize_notifications?}`; errors `not_found`, `finished`, `unavailable`, `needs_framed` | [hcS] 2272, 4 |
| `terminal.kill` | `{"terminal":"t7"}` | `Terminal` | [hcS] 1980 |
| `terminal.rm` | `{"terminal":"t7"}` | `{removed}`; error `live` if still running | [hcS] 2037 |
| `terminal.send` | `{"terminal":"t3","text":"Owner answered D7: …\n"}` | `{turn, started?, status?, wait?}`; errors `blocked`, `finished`, `not_found`, `unavailable` | [hcS] 909, 2444 |
| `events.subscribe` | `{}` (all terminals) | `{seq, terminals:[Terminal], channel?}` | [hcS] 1063, 3516 |

Close tab = `terminal.kill` then `terminal.rm` (as [go] `Stop`, agents.go:686-700).

### C2. Framed streams ([hcA] 45-110)

| kind | name | direction and payload |
|---|---|---|
| 0 | DATA | channel 0: one JSON-RPC message; stream channel: core → bytes of first paint then output; viewer → key bytes |
| 1 | CREDIT | viewer → core: u32 LE bytes consumed. A stream starts with 256 KiB credit; > 4 MiB behind gets a repaint |
| 2 | CLOSE | viewer → core: detach (core answers empty CLOSE). core → viewer: JSON reason, e.g. `{"reason":"exited","exit_code":0,"signal":null}` |
| 3 | RESIZE | viewer → core: u16 LE cols, u16 LE rows |
| 4 | FOCUS | viewer → core: u8 1 gained / 0 lost |
| 5 | FACTS | not sent by contr-rune (first version) |
| 6 | AUTOMATIC_INPUT | not sent (send DATA); only allowed when `hello.features` and the attach reply both have `automatic_input` |

Re-attach rules ([hcA] 160-170; [go] D79): a lost line → retry with `resumed:true`, backoff 0.5 s doubled to 5 s;
CLOSE with a final reason or attach error `finished`/`not_found`/`needs_framed` → no retry until a `restarted`
event for that terminal; `unavailable` → retry after `restarted`.

### C3. `Terminal` record (fields read; [hcS] 1134, `Status` 948, `Foreground` 276)

```json
{"address":"t17","name":"b-241","parent":"t3","cwd":"/…","argv":["claude","--model","sonnet"],
 "labels":{"contr":"agent","role":"builder","project":"contr"},
 "state":"live","turn":4,"title":"…","exit_code":null,"reason":null,"created_ms":0,"status_ms":0,"cols":120,"rows":40,
 "foreground":{"pid":1,"name":"claude","argv":[],"cwd":"/…"},
 "status":{"state":"working","kind":null,"msg":"","title":"","app":"claude-code","progress":null}}
```
`status` is null when never reported and after exit; `status.kind` is `permission|question|auth` when blocked.

### C4. Notifications ([hcA] 113-178)

- `{"method":"event","params":{"seq":N,"type":T,"terminal":"t7","data":{…}}}`. Apply:
  `created`/`restarted` = whole Terminal (add/replace; `restarted` → re-attach); `finished` = `{reason, exit_code,
  signal, finished_ms}` → `state:"finished"`, drop `foreground`; `lost` = same fields, stays live, drop `foreground`;
  `removed` = `{}` → drop; `status` = `{status, records, reported}`; `turn` = `{turn, state}`; `title` = `{title}`;
  `labels` = `{labels, record?}`; `size` = `{cols, rows}`; `foreground` = Foreground or null; `keep` = `{keep}`;
  `doc` = `{name, version}` (no terminal; ignored).
- `{"method":"heartbeat","params":{"seq":N}}` every 15 s.
- `{"method":"missed","params":{"seq":N,"terminals":[…]}}`: replace the list; drop events with seq ≤ N.
- `seq` restarts with each core: after a reconnect take a fresh snapshot.

### C5. Labels

Written by contr-rune on its tabs (same as [go] D82, so both apps show the same terminals):
`contr=agent`, `role=terminal`, `project=<project folder name>`, `worktree=<start folder>`.
Read on agents ([go] agents.go:26-50): `role` (`orchestrator`, `terminal` or a worker profile), `issue`, `profile`,
`worktree`, `project`, `contr.model`, `contr.effort`, `contr.orchestrating`.
A terminal without `contr=agent` is foreign: orchestrator if Claude runs in it (foreground name `claude` or a version
like `2.1.30`, or argv[0] `claude`, or it has a status), else plain ([go] agents.go:342-379).

### C6. Derived `Agent` (`src/hc/model.rs`, pure)

`address`, `name` (else address), `parent`, `role`, `project` (label `project`, else the contr-rune project whose
path prefixes `cwd` or `labels.worktree`), `model` (argv `--model`, else `contr.model`), `effort` (same), `title`,
`age_ms` (now − last event stamp for the terminal, else `status_ms`), `state`:
`NeedsYou` if status.state == `blocked` (keep `kind`, `msg`) → `Working` if `working` → `Waiting` if an orchestrator
has a live child that is working → `Idle` otherwise → `Ended` if `state == finished`. Needs-you wins
([go] `docs/spec/StatusGlyph.md`:20-32). Rows with `role=terminal` are tabs, not agent rows.

### C7. Owner items (shared with Go contr, decision O2)

- Path: `CONTR_OWNER_FILE`, else `~/Library/Application Support/contr/owner.json` ([go] `internal/owner/owner.go`:65-71).
- Shape (owner.go:20-56):
  `{"questions":[{"id":"D1","project","coordinator":"t3","from","title","question","options":[],"recommended":0,"impacts":[],"refs":[],"source","asked":RFC3339,"answer","answered_by","answered"}],"todos":[{"id":"T1","project","coordinator","from","kind":"do|know","text","pinned","done"}],"next_question":1,"next_todo":1}`.
  Missing file = empty. Unknown fields must be kept when writing (Go writes the full struct; keep a `serde_json::Value` round trip).
- Write: exclusive `flock` on `<path>.lock`, read, change, write `<path>.tmp` (0600), rename (owner.go:103-133).
- Answer: set `answer`, `answered_by:"owner"`, `answered` (owner.go:179-202), then `terminal.send` to
  `coordinator` with `"Owner answered <ID>: <answer>\n"`. To-do done: set `done`, send `"Owner did <ID>: <text>\n"`
  ([go] `internal/app/agentstate/model.go`:265-306). Send failure keeps the file change and shows
  `answer saved; coordinator not reached: <error>` (model.go:309-).
- `trait OwnerSource { fn read(&self) -> io::Result<OwnerState>; fn answer(&self, id, text) -> io::Result<Option<String /*coordinator*/>>; fn done(&self, id) -> io::Result<Option<String>>; fn changed_since(&self, stamp) -> bool; }`;
  first impl `OwnerFile`.
- Notch items: blocked agents, then open questions, then open to-dos of the selected project (needs.go:82-112).
  Label: "1 question", "N questions", or "N items" when a to-do or a blocked agent is among them ("1 item") (notch.go).
- Outline ticks: open to-dos of the orchestrator's project, oldest first; headline = first non-empty line; detail =
  second line, else "to do"/"to know" + " · from <from>"; width `min(14, 8 + chars/30)`; at most 40; inset 10,
  hit 20×10, pitch 11 ([go] `internal/app/outline.go`:42-58, 73-100).

### C8. Usage files (decision O5; same files as Go contr)

- State dir: `CONTR_STATE_DIR`, else `~/Library/Application Support/contr` ([go] `internal/usage/model.go` `StateDirFrom`).
- `<state>/usage/claude.json`: `{"schema":1,"updated_at":<unix s>,"claude_version":"…"|null,"windows":[{"kind":"five_hour"|"seven_day","used_percent":38.0,"resets_at":<unix s>|null}]}`
  from status-line input `rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}` (claude.go:24-90). Write only
  when windows change; temp file + rename.
- `<state>/usage/context/<id>.json` (id = `HC_TERMINAL_ID`, chars outside `[A-Za-z0-9_-]` → `_`):
  `{"schema":1,"updated_at","tokens","size","model","effort"}`; tokens = `context_window.current_usage.{input_tokens +
  cache_creation_input_tokens + cache_read_input_tokens}`, size = `context_window.context_window_size`, model = short
  family word of `model.id`/`display_name`, effort = `effort.level` (context.go:18-136). Label `"90k of 200k  45%"`.
- Codex: newest `rollout-*.jsonl` in the 2 newest day folders of `$CODEX_HOME/sessions` (default `~/.codex`), last
  256 KiB; last `token_count` line with `rate_limits` whose `limit_id` is absent or `codex` and `primary` has a
  percent; `primary` → kind `primary`, `secondary` → `secondary`; reset = `resets_at`, else file mtime +
  `resets_in_seconds` (codex.go:20-110). Read only that file.
- Percent left = 100 − highest used percent; a window past `resets_at` counts as 0; no data shows "–".

### C9. contr-rune switches, fake core, keys

- `CONTR_HC=<path>` program; `CONTR_HC=off` hc off (local PTY tabs).
- `CONTR_HC_FAKE=<fixture.json>`: in-process fake core. `CONTR_HC_FAKE_LOG=<file>`: the fake appends each mutating
  call (`terminal.run`, `terminal.send`, `terminal.kill`, `terminal.rm`, `terminal.attach` (H4)) and each stream input frame as a JSON line
  `{"method":…,"params":…}` / `{"frame":"data|resize|close|focus","terminal":…,"bytes":"<utf8>"|"cols","rows"|"gained"}`.
  H4: a timed fixture event with `"method":"fake"` is a control event: `{"drop":"framed"}` cuts every framed line,
  `{"refuse_attach_ms":N}` makes `terminal.attach` fail with kind `busy` for N ms. Against the fake the re-attach
  backoff is 50 ms doubled to 200 ms (`Dialer::backoff`), not 0.5 s to 5 s.
- Headless runs (`--snap`, `--tree`, `--script`) never dial a real hc: without `CONTR_HC_FAKE` they use local PTYs.
- Fixture: `{"hello":{…},"status":{…},"terminals":[Terminal…],"screens":{"t3":"<first paint text with \r\n>"},"events":[{"after_ms":300,"method":"event","params":{…}}],"errors":{"terminal.send":{"kind":"blocked","message":"…"}},"echo":true}`.
  The fake's `terminal.run` makes `t<next>` with the given labels and screen `"$ "`; with `echo` it writes stream
  DATA input back as output. Fixtures live in `tests/fixtures/hc/`.
- Element keys: `version/hc/value`; tabs keep `tab/<i>` and `term-<id>`; `agent/<address>`, `agent/<address>/glyph`,
  `agent/<address>/meta`, `agent/<address>/age`; `notch`, `notch/label`; `qcard`, `qcard/item/<ID>`,
  `qcard/option/<ID>/<i>`, `qcard/input`, `qcard/send`, `qcard/done/<ID>`, `qcard/error`; `outline`,
  `outline/tick/<ID>`, `outline/count`, `outline/card`; `meter/context`, `meter/claude`, `meter/codex`; `hc-banner`
  (connection problem line).

## 2. Rust design (`src/hc/`)

- Deps: add `serde` (derive) and `serde_json` to `[dependencies]` (now only a dev-dependency).
- `wire.rs`: serde types for C1-C4 (`Terminal`, `Status`, `Foreground`, `Snapshot`, `Event`, `RpcError`, `HelloResult`).
- `conn.rs` (plain): `Conn::new(reader, writer, notes: Option<Sender<Note>>)`; reader thread `hc-read` routes replies
  by id to `mpsc::Sender`s and notifications to `notes`; `call(method, params, timeout) -> Result<Value, HcError>`.
  `HcError::{NotFound, NotSent, Closed, Rpc{kind,message,hint}, Io}`. A call on a closed conn is `NotSent`
  (safe to retry; [go] D87); a conn that dies while waiting is `Closed` (do not retry a `terminal.run`).
- `framed.rs`: `encode(channel, kind, payload)`, `Decoder` (incremental, ≤ 16 MiB), `FramedConn`: reader thread
  `hc-frames` routes channel 0 JSON-RPC like `Conn` and stream channels to per-channel `StreamSink`s; writer mutex.
- `dial.rs`: `enum Dialer { Command{path, env}, Fake(Arc<Fixture>), Off }` → `(Box<dyn Read+Send>, Box<dyn Write+Send>)`.
  `Command` spawns `hc connect` (stderr tail kept for messages). `Fake` uses `UnixStream::pair()` and `fake::serve`.
- `fake.rs`: in-process fake core speaking plain and framed mode per C9.
- `watch.rs`: one `hc-watch` thread: dial → `hello` → `server.status` → second conn `events.subscribe {}` → apply C4 to
  `HashMap<String, Terminal>` + last-event stamps → send `HcUpdate::{Status(HcStatus), Terminals(..)}` over mpsc and
  call the Rune `Waker`. Reconnect: 0.5 s doubled to 5 s. `HcStatus::{Off, NotInstalled, Connecting, Connected{version}, Failed(String)}`.
- `term.rs`: `HcSession` with the same surface as `session::Session` (`term: Arc<Mutex<Term<_>>>`, `dirty`, `exited`,
  `last_output`, `generation`, `write`, `resize`, `scroll`, `tab_title`, `subtitle`). Its stream thread feeds
  `alacritty_terminal` exactly as `src/session.rs`:110-141 does, sends CREDIT after each batch, wakes the window once
  per drawn batch. `Tab.session` becomes `enum Pty { Local(Session), Hc(HcSession) }` (or a small trait).
- `model.rs` (C6), `owner.rs` (C7), `usage.rs` (C8): pure parsing with unit tests; file reads on threads.
- App: `hc: HcState`, `hc_rx: Receiver<HcUpdate>`, `selected_agent: Option<String>`. `setup()` starts the watcher unless
  headless/off; `poll()` drains `hc_rx` like `poll_diff` (`src/main.rs`:497-); `tx.wake_after(60 s)` only while an age
  shows. Scenes seed state synchronously from the fixture (`fake::seed`), as `refresh_diff_now` does (`src/main.rs`:331).
- hc calls from the UI (run, kill, send) go to a short-lived thread with its own connection and report back on `hc_rx`;
  never block the UI thread.

## 3. Parts (build order)

Every part: branch `part-h<n>-<name>` off `origin/main`; `cargo build` with no warnings; `cargo test`; snap
`cargo run -- --snap docs/design/snaps/partH<n>.png --size 1710x1112 --scale 1 --scene <scene>` (with
`CONTR_HC_FAKE=…` where the part needs it); tree and script checks become a test under `tests/`.

**H1 · Plain client, fake core, hc version in Settings**
- Files: `Cargo.toml`; new `src/hc/{mod,wire,conn,dial,fake,watch}.rs`; `src/main.rs` (state, setup, poll);
  `src/dialog.rs`:74 (`version/hc` value). Commit `docs/design/hc/{hc.openrpc.json,API.md}` and this plan.
- Done when Settings shows the core version (`0.1.19`) or "not installed" / "connecting" / "not running: <error>",
  and no headless run dials a real hc.
- Tests: unit: conn reply/notification routing, error kind, `NotSent`, generation refusal, `find_hc` order (temp
  dirs). `tests/hc.rs`: `CONTR_HC_FAKE=tests/fixtures/hc/basic.json`, scene `settings`, `--tree` →
  `version/hc/value == "0.1.19"`; without the fake → not a version. Owner check: `cargo run`, Settings shows 0.1.19.

**H2 · Framed codec and fake framed core** (no UI)
- Files: `src/hc/framed.rs`, `src/hc/fake.rs` (framed mode, `terminal.run/attach/kill/rm`, echo, log).
- Done when a test can `hello framed` → `terminal.run` → `terminal.attach` → read first paint → send DATA, RESIZE,
  CREDIT → get CLOSE with a reason, all against the fake.
- Tests: unit only: frame round trip, partial reads, 16 MiB limit, channel routing, CREDIT accounting, CLOSE JSON.
  Optional `#[ignore]` test against a scratch core (`HC_SESSION=crune-<pid>`, scratch XDG dirs under `/tmp/crune*`,
  `hc server stop` with that env on drop; never the owner's session).

**H3 · Tabs are hc terminals**
- Files: new `src/hc/term.rs`; `src/main.rs` (`open_tab`, `close_tab`, resize, keys, paste, poll); `src/session.rs`
  only to share the emulator feed.
- With hc connected: ⌘T, `+` and split open a tab with `terminal.run` (C1 params, size = pane grid) and attach it;
  keys/paste go as DATA, grid changes as RESIZE, focus as FOCUS; ⌘W kills and removes; a CLOSE `exited` closes the
  tab (today's "close on exit"). Find, scrollback, split keep working (they read the local emulator). With hc off or
  not installed: local PTYs as today.
- Done when the owner opens, types in, splits and closes tabs on the real core, and `hc ls` shows them with the C5 labels.
- Tests: `tests/hc_tabs.rs` with the fake + log: script ⌘T, type `ls\r`, ⌘W → log has `terminal.run` with the labels,
  DATA `ls\r`, `terminal.kill`, `terminal.rm`; tree shows the echoed text in the grid. Existing tests still pass on
  local PTYs.

**H4 · Restore and recover hc tabs**
- Files: `src/hc/term.rs`, `src/hc/watch.rs`, `src/main.rs`.
- At launch, live terminals with `contr=agent`, `role=terminal` and a `project` matching an open project become tabs
  (in `created_ms` order) and attach with `history:1000`. A lost line re-attaches with `resumed:true` (backoff per
  C2); a `restarted` event re-attaches; `finished` closes the tab. A banner `hc-banner` shows while the core is gone.
- Done when quitting and restarting contr-rune brings the same tabs back with their scrollback.
- Tests: fixture with two `role=terminal` terminals → tree has two tabs with their screens; fixture event
  `restarted` → log shows a second attach; event `finished` → tab gone.

**H5 · Agents in the sidebar**
- Files: `src/hc/model.rs`, `src/sidebar.rs`, `src/main.rs` (scene `agents`).
- Under each project: orchestrators, then their children (depth 2), as `examples/kit_preview.rs`:243-249
  `agent_row`: title, line 2 "b-241  sonnet" or "2 workers"/"orchestrator", age right ([go] spec AgentRow.md:18).
  Click selects (`selected_agent`).
- Done when the snap matches the agent rows of `docs/design/explore/kit-neutral.png`.
- Tests: unit for C6. `tests/hc_agents.rs`: order, depth, meta text, bounds vs `kit_preview --tree`; tap `agent/t17` selects it.

**H6 · Live agent state and needs-you**
- Files: `src/hc/watch.rs` (C4 apply in place), `src/hc/model.rs`, `src/sidebar.rs`.
- Kit `Spinner`/`RowStatus` per state; red only for needs-you; Claude mark for orchestrators; ages move once a minute.
- Done when a blocked agent shows needs-you after one event.
- Tests: unit per event type, `missed`, seq. Fixture `blocked.json` with timed events → after ~1 s the tree shows
  working on `t18`, needs-you on `t19`.

**H7 · Status probe (owner check, O3)**
- Files: `scripts/hc-status-probe.sh` only. It uses `HC_SESSION=crune-probe` and scratch XDG dirs, runs
  `hc run --bg --name probe-claude claude` and `hc run --bg --name probe-codex codex --no-daemon`, waits, prints
  `hc get` `status`/`reported` for each, then `hc server stop` for that session only.
- Done when the owner runs it and the result (status reported or not, the `app` value) is recorded in this file.
- If a program reports nothing: add **H7b · status hook** (owner approves first): a Claude Code hook
  (`UserPromptSubmit` → working, `Stop` → idle, `Notification` permission → blocked) that writes OSC 7501 to the
  terminal. The exact OSC 7501 byte format must be taken from the protocol spec first; it is not in the hc docs.

**H8 · Selecting an agent shows its terminal**
- Files: `src/main.rs`, `src/hc/term.rs`.
- Clicking an agent row attaches its hc terminal in the focused pane (as a tab of its project); the tab strip and
  sidebar follow, as with tabs today.
- Done when clicking the orchestrator row shows its live Claude session.
- Tests: fixture with `screens.t3` → tap `agent/t3` → grid tree has that text.

**H9 · Questions notch and card**
- Files: new `src/hc/owner.rs` (`OwnerSource`, `OwnerFile`), new `src/notch.rs`, `src/main.rs`.
- Notch hangs from the top of the terminal panel; click or ⌘J opens the card (rune-kit `Popover`): blocked agents,
  open questions (options, recommended marked, free text), to-dos (Done). Answer per C7. The file is re-read when its
  mtime changes (a thread checks while the window is focused; no work while idle).
- Done when an answer from the card reaches the coordinator terminal and Go contr's file shows it answered.
- Tests: unit: file round trip keeps unknown fields, lock, Decide errors, notch labels. `tests/hc_questions.rs`:
  `CONTR_OWNER_FILE`=temp copy, fake + log; tap `notch` → `qcard/option/D7/1` → `qcard/send` → file answered,
  log has `terminal.send` with `Owner answered D7: …\n`, count drops; fake `blocked` error → `qcard/error`.

**H10 · Orchestrator outline**
- Files: new `src/outline.rs`.
- Ticks on the right edge of the shown orchestrator terminal per C7; hover shows the card (headline, detail, clock
  time); new-count above the ticks.
- Done when the snap shows ticks on the orchestrator terminal.
- Tests: unit for ticks; tree has `outline/tick/T1…`; hover script shows `outline/card`.

**H11 · Claude and Codex plan meters**
- Files: new `src/usage.rs`; meter block in `src/files.rs`.
- Reads C8 files on a thread every 60 s while focused; "–" with no data.
- Done when the meters show real values on this Mac (Codex at once; Claude once H12 runs).
- Tests: unit for parsing and the reset rule; integration with `CONTR_STATE_DIR` and `CODEX_HOME` temp fixtures →
  `meter/claude`, `meter/codex` values.

**H12 · `contr-rune statusline` subcommand**
- Files: `src/main.rs` (argument check before Rune starts), `src/usage.rs` (writers).
- `contr-rune statusline` reads Claude Code's status-line JSON on stdin (≤ 1 MiB), writes C8 `usage/claude.json`
  and, when `HC_TERMINAL_ID` is set, `usage/context/<id>.json`; prints a one-line status. Owner check: set Claude
  Code `statusLine.command` to `contr-rune statusline`.
- Done when the Claude meter moves after a Claude turn.
- Tests: unit with sample status-line JSON → file contents; "same windows → no write"; oversize input → nothing written.

**H13 · Context meter**
- Files: `src/usage.rs`, `src/files.rs`.
- Shows the context file of the selected agent: "90k of 200k  45%" (custom label; kit `Progress` shows only `NN%`,
  `docs/design/explore/rune-kit.md`:47).
- Tests: select `agent/t3` with a fixture context file → `meter/context` value.

## 4. Dependencies

| Slice | Dependency |
|---|---|
| H1 | independent (first) |
| H2 | stacks on H1's branch (needs its code, not its merge) |
| H3 | needs H1 and H2 merged |
| H4 | stacks on H3's branch |
| H5 | needs H1 merged (can run beside H2-H4; small `main.rs`/`sidebar.rs` rebase) |
| H6 | stacks on H5's branch |
| H7 | independent (script only; owner runs it once hc is installed) |
| H8 | needs H3 and H5 merged |
| H9 | needs H1 merged |
| H10 | needs H8 and H9 merged |
| H11 | independent |
| H12 | independent (only `main.rs` argument check and `usage.rs`; merge after H11 if both touch `usage.rs`) |
| H13 | needs H5 and H11 merged |

## 5. Open questions (owner)

- O3 remains: run H7. If Claude Code or Codex reports no status, approve H7b (a hook in the owner's Claude settings).
- `contr-rune statusline` and an existing `statusLine` command: should it chain the old command (Go contr does, with a
  2 s timeout; claude.go:27-37) or replace it?
- Go contr and contr-rune label tabs the same way (C5), so each app shows the other's plain tabs. Keep, or add a
  label such as `contr.app=rune`?
