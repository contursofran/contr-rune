#!/usr/bin/env bash
# Bump the version, commit "Release v<version>" and tag it. Does not push.
# Usage: scripts/release.sh <version>   (for example 0.1.0)
set -euo pipefail

if [ $# -ne 1 ] || ! [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <version>   (for example 0.1.0)" >&2
  exit 2
fi
version="$1"
tag="v$version"

export PATH="$HOME/Library/Application Support/mbx/bin:$PATH:$HOME/.cargo/bin"
cd "$(cd "$(dirname "$0")/.." && pwd)"

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "not on main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "working tree is not clean" >&2; exit 1; }
git fetch origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "main is not up to date with origin/main" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists" >&2
  exit 1
fi

# Only the first `version = ` line (the [package] one) changes.
VERSION="$version" perl -0pi -e 's/^version = "[^"]*"/version = "$ENV{VERSION}"/m' Cargo.toml
grep -q "^version = \"$version\"" Cargo.toml || { echo "failed to set version in Cargo.toml" >&2; exit 1; }

cargo update -p contr-rune --offline

git add Cargo.toml Cargo.lock
git commit -m "Release $tag"
git tag "$tag"

echo "Tagged $tag. Push with:"
echo "  git push origin main $tag"
