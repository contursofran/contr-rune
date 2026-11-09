#!/bin/sh
# H7 status probe: do Claude Code and Codex report OSC 7501 status inside hc?
# Runs in its own hc session (HC_SESSION=crune-probe) with scratch runtime/state dirs, so the
# owner's real core and terminals are never touched. Stops that session's core at the end.
set -u
hc=${CONTR_HC:-$HOME/.local/bin/hc}
# Short path: macOS allows only 103 bytes for a unix socket path.
scratch=$(mktemp -d /tmp/crp.XXXX)
export HC_SESSION=crune-probe XDG_RUNTIME_DIR="$scratch/run" XDG_STATE_HOME="$scratch/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME"
cwd=${1:-$PWD}
wait_s=${PROBE_WAIT:-20}

"$hc" run --bg --no-parent --name probe-claude --cwd "$cwd" claude >/dev/null
"$hc" run --bg --no-parent --name probe-codex --cwd "$cwd" codex --no-daemon >/dev/null
echo "waiting ${wait_s}s for both programs to start and report…"
sleep "$wait_s"
# Optional: give Claude one tiny prompt so it has a turn to report (uses a little quota).
if [ -n "${PROBE_SEND:-}" ]; then
  "$hc" send probe-claude "$PROBE_SEND" >/dev/null
  sleep 3; echo "== probe-claude while answering"
  "$hc" get --json probe-claude | jq '{status}'
  sleep "$wait_s"
fi
for t in probe-claude probe-codex; do
  echo "== $t"
  "$hc" get --json "$t" | jq '{state, title, status, foreground: .foreground.name}'
done
"$hc" server stop >/dev/null 2>&1
rm -rf "$scratch"
