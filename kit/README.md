# contr kit

A Claude Code plugin (name `contr`) for agent work with `contr-rune agents`:

- `agents/`: the builder, planner and reviewer agent definitions.
- `commands/orchestrate.md`: the coordinator playbook, as `/contr:orchestrate <goal>`.
- `hooks/hooks.json`: the Stop hook (a worker's turn that sent no report). Later parts add the
  SessionStart, PostToolUse and UserPromptSubmit hooks.
- `.mcp.json`: starts `contr-rune mcp`, which gives the session the agent tools (spawn, message,
  interrupt, stop, read, list, clear_ended). `contr-rune` must be on the PATH; `scripts/install-cli.sh`
  links it.

## Install

See `docs/KIT.md`. The repository root is a plugin marketplace (`.claude-plugin/marketplace.json`, name
`contr-rune`): `claude plugin marketplace add <repo>`, then `claude plugin install contr@contr-rune`.

## Use

1. `contr-rune agents` finds this folder through `CONTR_KIT`, the `kit` key of the profiles file, or the
   copy in the app bundle (`Contents/Resources/kit`). A claude profile with an agent file here starts as
   that agent: `claude --plugin-dir <kit> --agent contr:<profile> …`.
2. Start the coordinator in an hc terminal with the plugin, then `/contr:orchestrate <goal>`.
3. Workers that the coordinator starts get its address in `CONTR_COORDINATOR` and report with `hc send`.
   When a worker's turn ends without one, the Stop hook (`contr-rune hook stop`) sends the coordinator
   "ended its turn without a report. Last words: …". The hook does nothing in sessions without
   `CONTR_COORDINATOR`.
