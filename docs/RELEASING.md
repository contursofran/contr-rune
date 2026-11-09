# Releasing

- Tag format: `v<version>`, for example `v0.1.0`. It must equal the `Cargo.toml` version (`0.1.0`); CI fails if not.
- Run `scripts/release.sh <version>` on a clean, current `main`. It bumps `Cargo.toml` and `Cargo.lock`, commits
  "Release v<version>" and tags. Then push: `git push origin main v<version>`.
- CI (`.github/workflows/release.yml`) builds on an Apple Silicon runner with `scripts/bundle-app.sh` and publishes a
  GitHub Release with these assets. The in-app updater downloads exactly these names:
  - `contr-rune-macos-arm64.zip` (`ditto -c -k --keepParent` of `contr-rune.app`)
  - `contr-rune-macos-arm64.zip.sha256` (`shasum -a 256` output)
- The app is ad-hoc signed, not notarised. The release notes say how to open it the first time.
- `workflow_dispatch` runs a dry build without publishing.
