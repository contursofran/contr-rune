# contr-rune

A small port of [contr](https://github.com/contursofran/contr)'s sidebar, terminal tabs and terminal
to [Rune](https://github.com/anomalyco/rune), a GPU-rendered Rust UI framework. It's an experiment to
see how Rune compares with GPUI and MyGo for this kind of app, not a replacement for contr.

![contr-rune: projects with their terminals, tabs and a shell](docs/screenshot.png)

- **Sidebar:** each project with its terminals under it, and a `+` to open one in that folder. Below,
  the terminals grouped by state: **Working** (amber dot, output in the last 1.5 s) and **Idle**
  (hollow ring, with how long it's been quiet).
- **Tabs:** the selected project's terminals, labelled `$ <folder>` from the shell's title, with a
  working dot and `×`, plus `+` for a new one.
- **Terminal:** a real login shell in a PTY, parsed by `alacritty_terminal` and drawn as one coloured
  text row per line. Scrollback with the wheel or Shift+PageUp/PageDown. A tab closes when its
  shell exits.

Colours and type follow contr's tokens (`docs/DESIGN.md` in contr).

## Run

```sh
cargo run                                     # projects: the current folder and home
CONTR_PROJECTS=~/proj-a:~/proj-b cargo run    # or pick your own
```

Rust 1.99.1 (pinned in `rust-toolchain.toml`). On Linux, Rune needs `libxkbcommon` and `wayland`
at build time.

Rune is pre-release, so `Cargo.toml` pins a commit of its `dev` branch.

## Keys

| Keys (Linux) | macOS | Action |
|---|---|---|
| Ctrl+Shift+T | ⌘T | New tab in the selected project |
| Ctrl+Shift+W | ⌘W | Close tab |
| Ctrl+Tab, Ctrl+PageDown | | Next tab |
| Ctrl+Shift+Tab, Ctrl+PageUp | | Previous tab |
| Ctrl+Shift+V | ⌘V | Paste (bracketed when the program asks) |
| Shift+PageUp / PageDown | | Scroll back |

Everything else, Ctrl keys included, goes to the shell.

## Check without a window

Rune renders headless, so the UI can be checked without opening a window:

```sh
cargo test                                              # key encoding tests
cargo run -- --snap out.png --scene three-tabs --at 1   # three shells, rendered to PNG
```

## Layout

| File | What |
|---|---|
| `src/main.rs` | State, the sidebar, tabs and terminal views, keys and shortcuts |
| `src/session.rs` | A shell in a PTY; a reader thread feeds `alacritty_terminal` and wakes the window |
| `src/grid.rs` | The emulator's visible grid → text rows with colour spans and the cursor |
| `src/keys.rs` | Rune key events → xterm byte sequences |
| `src/theme.rs` | contr's colour and type tokens |

## Not built

- Text selection and copy, and mouse reporting to programs inside the terminal.
- Measured cell width: it assumes 0.6 em, which fits JetBrains Mono and DejaVu Sans Mono.
- herdr (`hc`) agents: these are plain shells. Projects aren't saved between launches.
