#!/usr/bin/env bash
# Build target/bundle/contr-rune.app (release, ad-hoc signed).
# Usage: scripts/bundle-app.sh [--install]
#   --install  also copy the app to ~/Applications/contr-rune.app
set -euo pipefail

install=0
case "${1:-}" in
  "") ;;
  --install) install=1 ;;
  *) echo "usage: $0 [--install]" >&2; exit 2 ;;
esac

export PATH="$HOME/Library/Application Support/mbx/bin:$PATH:$HOME/.cargo/bin"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

name=contr-rune
version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

nice -n 10 cargo build --release

app="$root/target/bundle/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/$name" "$app/Contents/MacOS/$name"
cp "$root/assets/icon/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

# The Claude Code plugin (agents, playbook, hooks); `agents::spawn` looks for it here.
cp -R "$root/kit" "$app/Contents/Resources/kit"

# Fonts are embedded with include_bytes!, so nothing else is copied.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>dev.contursofran.contr-rune</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleExecutable</key><string>$name</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$app"
echo "built $app"

if [ "$install" = 1 ]; then
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/$name.app"
  cp -R "$app" "$HOME/Applications/$name.app"
  echo "installed ~/Applications/$name.app"
fi
