#!/usr/bin/env bash
# Link ~/.local/bin/contr-rune to the binary inside the app bundle.
# Usage: scripts/install-cli.sh [--uninstall] [--app <path to contr-rune.app>]
#   --uninstall  remove the link (only if it is a symlink)
#   --app PATH   bundle to link to (default: ~/Applications/contr-rune.app, else target/bundle/contr-rune.app)
set -euo pipefail

usage() { echo "usage: $0 [--uninstall] [--app <path to contr-rune.app>]" >&2; exit 2; }

root="$(cd "$(dirname "$0")/.." && pwd)"
link="$HOME/.local/bin/contr-rune"
uninstall=0
app=""
while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall) uninstall=1; shift ;;
    --app) [ $# -ge 2 ] || usage; app="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) usage ;;
  esac
done

if [ "$uninstall" = 1 ]; then
  if [ -L "$link" ]; then
    rm "$link"
    echo "removed $link"
  else
    echo "nothing to remove at $link"
  fi
  exit 0
fi

if [ -z "$app" ]; then
  if [ -d "$HOME/Applications/contr-rune.app" ]; then
    app="$HOME/Applications/contr-rune.app"
  else
    app="$root/target/bundle/contr-rune.app"
  fi
fi
bin="$app/Contents/MacOS/contr-rune"
[ -x "$bin" ] || { echo "no binary at $bin (run scripts/bundle-app.sh --install first)" >&2; exit 1; }

if [ -e "$link" ] && [ ! -L "$link" ]; then
  echo "$link exists and is not a symlink; not touching it" >&2
  exit 1
fi
mkdir -p "$(dirname "$link")"
ln -sfn "$bin" "$link"
echo "linked $link -> $bin"
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo "note: $HOME/.local/bin is not on your PATH" >&2 ;;
esac
