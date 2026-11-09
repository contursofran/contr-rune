#!/usr/bin/env bash
# Regenerate assets/icon/AppIcon.icns from assets/icon/contr-rune.svg (needs rsvg-convert + iconutil).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
set_dir="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$set_dir"
for s in 16 32 128 256 512; do
  rsvg-convert -w "$s" -h "$s" "$root/assets/icon/contr-rune.svg" -o "$set_dir/icon_${s}x${s}.png"
  rsvg-convert -w "$((s * 2))" -h "$((s * 2))" "$root/assets/icon/contr-rune.svg" -o "$set_dir/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$set_dir" -o "$root/assets/icon/AppIcon.icns"
