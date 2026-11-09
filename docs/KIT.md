# Install the kit

The kit (`kit/`) is a Claude Code plugin named `contr`: the builder, planner and reviewer agents, the
`/contr:orchestrate` playbook, the Stop hook and the `contr-rune mcp` server. The repo root is a plugin
marketplace named `contr-rune` (`.claude-plugin/marketplace.json`). The bundled app also carries a copy in
`contr-rune.app/Contents/Resources/kit`, which `contr-rune agents` uses for workers.

## Steps

1. Build and install the app, then link the CLI:
   `scripts/bundle-app.sh --install && scripts/install-cli.sh`
   (`~/.local/bin/contr-rune` must be on the PATH).
2. Remove Go contr's plugin first (two plugins named `contr` clash):
   `claude plugin uninstall contr@contr` then `claude plugin marketplace remove contr`.
3. Add this repo as a marketplace and install:
   `claude plugin marketplace add <path or URL of the contr-rune repo>`
   `claude plugin install contr@contr-rune`
4. In a Claude session, `/contr:orchestrate <goal>` starts a coordinator.

## Uninstall

`claude plugin uninstall contr@contr-rune`, `claude plugin marketplace remove contr-rune`,
`scripts/install-cli.sh --uninstall`.

Claude Code caches an installed plugin by its version. **Raise `version` in `kit/.claude-plugin/plugin.json` in
every PR that changes `kit/`**, or installed copies keep the old files. To pick up a new version:
`claude plugin marketplace update contr-rune`, then `claude plugin update contr@contr-rune`. The hooks call the
`contr-rune` binary, so also reinstall the app (`scripts/bundle-app.sh --install`) when hook commands change.
