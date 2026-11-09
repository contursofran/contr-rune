//! The update check (R3a): ask GitHub for the latest release with `/usr/bin/curl` on a thread, and
//! keep the newer version (if any) for the title-bar pill, the toast and the Settings row.
//! `CONTR_UPDATE_URL` points the check at another URL (a `file://` JSON in tests); a headless run
//! without it makes no check and never touches the network.
//!
//! The install (R3b, "Restart now"): download the zip, its `.sha256` and its `.sig` from the release,
//! check the sha256 (the download is whole) and the ed25519 signature (the release is ours), unzip
//! with `ditto`, then a detached `/bin/sh` helper (`scripts/update-helper.sh`) waits for this
//! process to exit, swaps the `.app` and opens it. Any error shows in Settings (`update/error`).
//! With `CONTR_UPDATE_URL` the assets come from the folder of that URL, `CONTR_UPDATE_PUBKEY` (hex)
//! replaces the public key, and `CONTR_UPDATE_DRY=1` stops after the unzip, before any swap.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use rune::{El, Tx, Waker, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Placement, Toast, ToastRegion, popover};

use crate::App;

const LATEST_URL: &str = "https://api.github.com/repos/contursofran/contr-rune/releases/latest";
const DOWNLOADS: &str = "https://github.com/contursofran/contr-rune/releases/download";
const ZIP: &str = "contr-rune-macos-arm64.zip";
/// The public key (ed25519, 32 bytes in hex) the release zips are verified with.
const PUBKEY: &str = "fb476085368fa7f9529fe4a7380b82038e1e712df208d398624e671ce58bcf1f";
/// The helper script, run by `/bin/sh -c` once this process has exited.
const HELPER: &str = include_str!("../scripts/update-helper.sh");
/// The running version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// How often the window checks while it is visible.
const EVERY: Duration = Duration::from_secs(15 * 60);
/// A check on focus happens when the last good one is older than this.
const STALE: Duration = Duration::from_secs(10 * 60);
/// "What's new" shows at most this many items.
const MAX_ITEMS: usize = 6;

/// A release newer than the running app.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// The version without the leading `v`.
    pub version: String,
    /// The list items of the release body.
    pub items: Vec<String>,
}

/// The latest release as the API gave it.
#[derive(Debug, Clone, PartialEq)]
struct Newest {
    tag: String,
    items: Vec<String>,
}

/// What one check found: `None` when it failed.
type Found = Option<Newest>;

/// The numbers of a version tag (`v1.2.3`, `1.2`, `1.2.3-rc1`): missing parts are 0.
fn parse_version(tag: &str) -> Option<[u64; 3]> {
    let core = tag.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut out = [0; 3];
    let mut parts = core.split('.');
    for slot in &mut out {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None => break,
        }
    }
    parts.next().is_none().then_some(out)
}

/// Whether `tag` is a newer version than `current`; an unreadable tag is not newer.
pub fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_version(tag), parse_version(current)), (Some(t), Some(c)) if t > c)
}

/// The list items (`- x` or `* x`) of a release body, GitHub's " by @user in <url>" tail removed,
/// at most `MAX_ITEMS`.
pub fn items(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| l.trim_start().strip_prefix("- ").or_else(|| l.trim_start().strip_prefix("* ")))
        .map(|item| {
            let item = item.trim();
            match item.rfind(" by @") {
                Some(i) if item[i..].contains(" in http") => item[..i].trim_end(),
                _ => item,
            }
            .to_owned()
        })
        .filter(|i| !i.is_empty())
        .take(MAX_ITEMS)
        .collect()
}

/// The newest release in the API's JSON: its tag and body items. Drafts and prereleases are skipped.
fn parse_release(json: &str) -> Option<Newest> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = v["tag_name"].as_str().filter(|t| parse_version(t).is_some())?;
    Some(Newest { tag: tag.to_owned(), items: items(v["body"].as_str().unwrap_or("")) })
}

/// The URL to check: `CONTR_UPDATE_URL`, else GitHub (not in a headless run).
fn url() -> Option<String> {
    match std::env::var("CONTR_UPDATE_URL") {
        Ok(u) if !u.is_empty() => Some(u),
        _ if rune::is_headless() => None,
        _ => Some(LATEST_URL.to_owned()),
    }
}

/// Fetch and read the latest release (K5: `/usr/bin/curl -fsSL`).
fn fetch(url: &str) -> Found {
    let out = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: contr-rune", url])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse_release(&String::from_utf8_lossy(&out.stdout))).flatten()
}

/// How far "Restart now" got.
#[derive(Debug, Clone, PartialEq)]
enum Install {
    Idle,
    Working,
    /// A dry run (`CONTR_UPDATE_DRY`) downloaded, verified and unzipped the update.
    Verified,
    Failed(String),
}

/// How an install ended without error.
#[derive(Debug, PartialEq)]
enum Installed {
    /// The helper runs: this app must quit now.
    Swapping,
    Verified,
}

fn hex32(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// The key to verify with: the embedded one (`CONTR_UPDATE_PUBKEY` replaces it only next to `CONTR_UPDATE_URL`).
fn pubkey() -> Result<[u8; 32], String> {
    let hex = match (std::env::var("CONTR_UPDATE_URL"), std::env::var("CONTR_UPDATE_PUBKEY")) {
        (Ok(u), Ok(k)) if !u.is_empty() && !k.is_empty() => k,
        _ => PUBKEY.to_owned(),
    };
    hex32(&hex).ok_or_else(|| "The update key is not valid.".to_owned())
}

/// Whether `sig` (the raw 64 bytes `openssl pkeyutl -sign -rawin` writes) is `key`'s signature of `data`.
fn verify_signature(key: &[u8; 32], data: &[u8], sig: &[u8]) -> Result<(), String> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let bad = || "The update's signature does not match; it was not installed.".to_owned();
    let sig = Signature::from_slice(sig).map_err(|_| bad())?;
    let key = VerifyingKey::from_bytes(key).map_err(|_| "The update key is not valid.".to_owned())?;
    key.verify_strict(data, &sig).map_err(|_| bad())
}

/// Whether the digest (hex) is the first word of a `shasum -a 256` line.
fn check_sha256(digest: &str, sha_file: &str) -> Result<(), String> {
    match sha_file.split_whitespace().next() {
        Some(want) if want.eq_ignore_ascii_case(digest) => Ok(()),
        _ => Err("The download is damaged (checksum mismatch); it was not installed.".to_owned()),
    }
}

/// The `.app` that holds this executable (`X.app/Contents/MacOS/bin`), if any.
fn app_path(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let app = macos.parent()?.parent()?;
    (macos.file_name()? == "MacOS" && app.extension()? == "app").then(|| app.to_path_buf())
}

/// Where the release assets are: the folder of `CONTR_UPDATE_URL`, else the release's download URL.
fn asset_base(update_url: Option<&str>, tag: &str) -> String {
    match update_url {
        Some(u) if !u.is_empty() => u.rsplit_once('/').map_or(u, |(dir, _)| dir).to_owned(),
        _ => format!("{DOWNLOADS}/{tag}"),
    }
}

/// The arguments of `/bin/sh` that run the helper for `pid`, the unzipped app, the installed app and the work folder.
fn helper_args(pid: u32, new_app: &Path, app: &Path, work: &Path) -> Vec<OsString> {
    vec!["-c".into(), HELPER.into(), "sh".into(), pid.to_string().into(), new_app.into(), app.into(), work.into()]
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let status = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "300", "-o"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("Could not download the update: {e}"))?;
    status.success().then_some(()).ok_or_else(|| format!("Could not download {url}."))
}

fn sha256_of(file: &Path) -> Result<String, String> {
    let out = Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(file).stdin(Stdio::null()).output().map_err(|e| format!("Could not check the download: {e}"))?;
    let line = String::from_utf8_lossy(&out.stdout);
    line.split_whitespace().next().filter(|_| out.status.success()).map(str::to_owned).ok_or_else(|| "Could not check the download.".to_owned())
}

/// Download, verify and unzip the release `tag` into `work`; returns the new app.
fn prepare(tag: &str, work: &Path) -> Result<PathBuf, String> {
    let io = |e: std::io::Error| format!("Could not prepare the update: {e}");
    let base = asset_base(std::env::var("CONTR_UPDATE_URL").ok().as_deref(), tag);
    let (zip, sha, sig) = (work.join(ZIP), work.join(format!("{ZIP}.sha256")), work.join(format!("{ZIP}.sig")));
    for (path, name) in [(&zip, ZIP.to_owned()), (&sha, format!("{ZIP}.sha256")), (&sig, format!("{ZIP}.sig"))] {
        download(&format!("{base}/{name}"), path)?;
    }
    check_sha256(&sha256_of(&zip)?, &std::fs::read_to_string(&sha).map_err(io)?)?;
    verify_signature(&pubkey()?, &std::fs::read(&zip).map_err(io)?, &std::fs::read(&sig).map_err(io)?)?;
    let unzipped = work.join("unzipped");
    let ok = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&unzipped).stdin(Stdio::null()).status().is_ok_and(|s| s.success());
    let app = unzipped.join("contr-rune.app");
    (ok && app.is_dir()).then_some(app).ok_or_else(|| "The update zip does not hold contr-rune.app.".to_owned())
}

/// The whole install of release `tag` (blocking): see the module doc.
fn install(tag: &str) -> Result<Installed, String> {
    let dry = std::env::var("CONTR_UPDATE_DRY").is_ok_and(|v| !v.is_empty());
    let exe = std::env::current_exe().map_err(|e| format!("Could not find the running app: {e}"))?;
    let app = app_path(&exe);
    if app.is_none() && !dry {
        return Err("contr-rune does not run from an app bundle, so it cannot replace itself. Download the update from GitHub.".to_owned());
    }
    let work = std::env::temp_dir().join(format!("contr-rune-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("Could not prepare the update: {e}"))?;
    let prepared = prepare(tag, &work);
    let (Some(app), Ok(new_app), false) = (app, &prepared, dry) else {
        let _ = std::fs::remove_dir_all(&work);
        return prepared.map(|_| Installed::Verified);
    };
    // The helper removes `work` when it is done.
    Command::new("/bin/sh")
        .args(helper_args(std::process::id(), new_app, &app, &work))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Could not start the updater: {e}"))?;
    Ok(Installed::Swapping)
}

/// The update state of the app.
pub struct Updater {
    /// The newer release found by the last good check.
    pub ready: Option<Release>,
    /// Some check has succeeded (Settings says "up to date" only then).
    checked: bool,
    last_try: Option<Instant>,
    last_ok: Option<Instant>,
    inflight: bool,
    /// The last check failed (no network, a bad answer).
    failed: bool,
    /// A check was asked for (focus, the Settings button) and waits for `poll`.
    wanted: bool,
    /// The version the toast was shown for: one toast per version.
    toasted: Option<String>,
    pub toast: bool,
    pub panel: bool,
    /// "Restart now" was chosen and waits for `poll` (it has the waker).
    restart_wanted: bool,
    install: Install,
    done_tx: Sender<Result<Installed, String>>,
    done_rx: Receiver<Result<Installed, String>>,
    tx: Sender<Found>,
    rx: Receiver<Found>,
}

impl Updater {
    pub fn new() -> Updater {
        let (tx, rx) = channel();
        let (done_tx, done_rx) = channel();
        Updater {
            ready: None,
            checked: false,
            last_try: None,
            last_ok: None,
            inflight: false,
            failed: false,
            wanted: false,
            toasted: None,
            toast: false,
            panel: false,
            restart_wanted: false,
            install: Install::Idle,
            done_tx,
            done_rx,
            tx,
            rx,
        }
    }

    /// Take a result in (and show the toast for a version not yet announced). Returns whether the view changes.
    fn accept(&mut self, found: Found) -> bool {
        self.inflight = false;
        let Some(newest) = found else {
            self.failed = true;
            return true;
        };
        self.failed = false;
        self.checked = true;
        self.last_ok = Some(Instant::now());
        let ready = is_newer(&newest.tag, CURRENT).then(|| Release { version: newest.tag.trim_start_matches('v').to_owned(), items: newest.items });
        let mut changed = ready != self.ready;
        match &ready {
            Some(r) if self.toasted.as_deref() != Some(r.version.as_str()) => {
                self.toasted = Some(r.version.clone());
                self.toast = true;
                changed = true;
            }
            Some(_) => {}
            None => {
                changed |= self.toast || self.panel;
                self.toast = false;
                self.panel = false;
            }
        }
        self.ready = ready;
        changed
    }

    /// Check now on this thread (a scene cannot wait for the thread).
    pub fn check_now(&mut self) {
        if let Some(url) = url() {
            self.last_try = Some(Instant::now());
            self.accept(fetch(&url));
        }
    }

    /// The window got the focus: check again when the last good check is old.
    pub fn on_focus(&mut self) {
        if self.last_ok.is_none_or(|t| t.elapsed() > STALE) {
            self.wanted = true;
        }
    }

    /// The Settings button.
    pub fn check_again(&mut self) {
        self.wanted = true;
    }

    /// Take a finished check and start the next one: at launch, every 15 minutes while the window
    /// has the focus, and when `wanted`. Returns whether the view changes.
    pub fn poll(&mut self, focused: bool, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(found) = self.rx.try_recv() {
            changed |= self.accept(found);
        }
        changed |= self.poll_install(waker, tx);
        let Some(url) = url() else { return changed };
        let due = self.last_try.is_none_or(|t| t.elapsed() >= EVERY);
        if !self.inflight && (self.wanted || (focused && due)) {
            self.wanted = false;
            self.inflight = true;
            changed = true;
            self.last_try = Some(Instant::now());
            let (out, waker) = (self.tx.clone(), waker.cloned());
            std::thread::spawn(move || {
                let _ = out.send(fetch(&url));
                if let Some(w) = &waker {
                    w();
                }
            });
        }
        tx.wake_after(EVERY);
        changed
    }

    /// "Restart now": the next `poll` starts the install.
    pub fn restart(&mut self) {
        self.restart_wanted = self.ready.is_some() && self.install != Install::Working;
        self.toast = false;
        self.panel = false;
    }

    /// Take the end of an install (quit when the helper runs) and start one that was asked for. A
    /// headless run cannot wait for a thread, so it installs at once.
    fn poll_install(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(done) = self.done_rx.try_recv() {
            changed = true;
            self.install = match done {
                Ok(Installed::Swapping) => {
                    tx.quit();
                    Install::Working
                }
                Ok(Installed::Verified) => Install::Verified,
                Err(e) => Install::Failed(e),
            };
        }
        if !std::mem::take(&mut self.restart_wanted) {
            return changed;
        }
        let Some(release) = &self.ready else { return changed };
        let tag = format!("v{}", release.version);
        self.install = Install::Working;
        if rune::is_headless() {
            let _ = self.done_tx.send(install(&tag));
            self.poll_install(waker, tx);
            return true;
        }
        let (out, waker) = (self.done_tx.clone(), waker.cloned());
        std::thread::spawn(move || {
            let _ = out.send(install(&tag));
            if let Some(w) = &waker {
                w();
            }
        });
        true
    }

    /// Why the last "Restart now" failed, for Settings.
    pub fn error(&self) -> Option<&str> {
        match &self.install {
            Install::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// The Settings value after the app's version.
    pub fn status(&self) -> String {
        match (&self.ready, self.checked) {
            (Some(r), _) if self.install == Install::Working => format!("{CURRENT} \u{b7} installing {}\u{2026}", r.version),
            (Some(r), _) if self.install == Install::Verified => format!("{CURRENT} \u{b7} {} verified (dry run)", r.version),
            (Some(r), _) => format!("{CURRENT} \u{b7} {} available", r.version),
            (None, _) if self.inflight => format!("{CURRENT} \u{b7} checking\u{2026}"),
            (None, _) if self.failed => format!("{CURRENT} \u{b7} could not check"),
            (None, true) => {
                let ago = self.last_ok.map_or(String::new(), |t| format!(" \u{b7} checked {}", ago(t.elapsed())));
                format!("{CURRENT} \u{b7} up to date{ago}")
            }
            (None, false) => CURRENT.to_owned(),
        }
    }
}

/// How long ago, for Settings: "just now", "5 min ago", "2 h ago".
fn ago(d: Duration) -> String {
    match d.as_secs() / 60 {
        0 => "just now".to_owned(),
        m @ 1..60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

/// Push the toast and the pill's popover onto the overlay `layer`.
pub fn overlays(k: &Kit, s: &App, layer: &mut Vec<El>) {
    let u = &s.update;
    let Some(release) = &u.ready else { return };
    let c = &k.c;
    if u.panel {
        let items = release.items.iter().enumerate().map(|(i, item)| {
            row()
                .key(format!("update-panel/item/{i}"))
                .w_full()
                .gap(8.0)
                .child(text("\u{2022}").font(k.ty.ui).color(c.text_faint))
                .child(text(item.clone()).font(k.ty.ui).color(c.text_base).wrap().grow(1.0))
        });
        let mut body = col()
            .w_full()
            .gap(8.0)
            .px(16.0)
            .pt(14.0)
            .pb(12.0)
            .child(text(format!("contr-rune {} is ready", release.version)).key("update-panel/version").font(k.ty.ui_medium).color(c.text_strong))
            .child(text(format!("You are on {CURRENT}. Agents keep running in hc.")).key("update-panel/from").font(k.ty.text_small).color(c.text_muted).wrap().w_full());
        if !release.items.is_empty() {
            body = body.child(text("What's new").font(k.ty.small_medium).color(c.text_faint).pt(4.0)).children(items);
        }
        let later = Button::new("Later").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).on_press(|s: &mut App| s.update.panel = false).build(k).key("update-panel/later");
        let restart = Button::new("Restart now").variant(ButtonVariant::Contrast).control_size(ControlSize::Small).on_press(|s: &mut App| s.update.restart()).build(k).key("update-panel/restart");
        let footer = row().w_full().px(16.0).pb(12.0).gap(8.0).child(spacer()).child(later).child(restart);
        let card = col().key("update-panel").role("dialog").label("Update ready").w(320.0).child(body).child(footer);
        layer.push(rune_kit::scrim("update-panel-scrim", |s: &mut App| s.update.panel = false));
        layer.push(popover::anchored(k, popover::raised(k, card, crate::theme::RADIUS_12), "update-pill", Placement::BottomEnd, 6.0));
    }
    if u.toast {
        let toast = Toast::new("update-toast", "Update ready")
            .description(format!("contr-rune {} is ready. Restart to finish updating.", release.version))
            .action(k, "Restart now", true, |s: &mut App, _tx| s.update.restart())
            .action(k, "Later", false, |s: &mut App, _tx| s.update.toast = false)
            .on_close(|s: &mut App| s.update.toast = false)
            .build(k);
        layer.push(ToastRegion::new().toast(toast).build(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_tags_compare_by_number() {
        for (tag, current, newer) in [
            ("v0.2.0", "0.1.0", true),
            ("v0.1.1", "0.1.0", true),
            ("v0.10.0", "0.9.0", true),
            ("v1.0", "0.9.9", true),
            ("v0.1.0", "0.1.0", false),
            ("v0.0.9", "0.1.0", false),
            ("0.2.0-rc1", "0.1.0", true),
            ("nightly", "0.1.0", false),
            ("v1.2.3.4", "0.1.0", false),
            ("", "0.1.0", false),
        ] {
            assert_eq!(is_newer(tag, current), newer, "{tag} vs {current}");
        }
    }

    #[test]
    fn body_becomes_at_most_six_items() {
        let body = "Intro\n\n## What's Changed\n* One by @me in https://github.com/x/pull/1\n- Two\n  * Three by @me in https://x\n* Fix by @ref of things\n* Five\n*\n* Six\n* Seven\n* Eight\n\n**Full Changelog**: https://x";
        assert_eq!(items(body), ["One", "Two", "Three", "Fix by @ref of things", "Five", "Six"]);
        assert!(items("no list here").is_empty());
    }

    #[test]
    fn release_json_gives_tag_and_items() {
        let n = parse_release(include_str!("../tests/fixtures/update/newer.json")).unwrap();
        assert_eq!(n.tag, "v9.9.9");
        assert_eq!(n.items.len(), 3);
        assert_eq!(n.items[0], "Update pill and toast");
        let real = parse_release(include_str!("../tests/fixtures/update/current.json")).unwrap();
        assert_eq!((real.tag.as_str(), real.items.len()), ("v0.1.0", 6));
        for bad in ["", "not json", "{}", r#"{"tag_name":"nightly"}"#, r#"{"tag_name":"v2.0.0","draft":true}"#, r#"{"tag_name":"v2.0.0","prerelease":true}"#] {
            assert_eq!(parse_release(bad), None, "{bad:?}");
        }
    }

    /// A key pair from a fixed seed (never the release key) and its signature of `data`.
    fn signed(data: &[u8]) -> ([u8; 32], Vec<u8>) {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&[7; 32]);
        (sk.verifying_key().to_bytes(), sk.sign(data).to_bytes().to_vec())
    }

    #[test]
    fn a_signature_must_match_the_zip_and_the_key() {
        let (key, sig) = signed(b"the zip");
        assert_eq!(verify_signature(&key, b"the zip", &sig), Ok(()));
        assert!(verify_signature(&key, b"the zip, tampered", &sig).is_err(), "tampered data");
        let mut flipped = sig.clone();
        flipped[10] ^= 1;
        assert!(verify_signature(&key, b"the zip", &flipped).is_err(), "tampered signature");
        assert!(verify_signature(&key, b"the zip", &[]).is_err(), "missing signature");
        assert!(verify_signature(&key, b"the zip", &sig[..63]).is_err(), "short signature");
        let other = hex32(PUBKEY).unwrap();
        assert!(verify_signature(&other, b"the zip", &sig).is_err(), "another key");
    }

    #[test]
    fn keys_are_64_hex_digits() {
        assert_eq!(hex32(PUBKEY).unwrap()[..2], [0xfb, 0x47]);
        for bad in ["", "zz", &PUBKEY[1..], &PUBKEY.replace('f', "g")] {
            assert_eq!(hex32(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_checksum_is_the_first_word_of_the_sha_file() {
        let line = "AbC123  contr-rune-macos-arm64.zip\n";
        assert_eq!(check_sha256("abc123", line), Ok(()));
        assert!(check_sha256("abc124", line).is_err());
        assert!(check_sha256("abc123", "").is_err());
    }

    #[test]
    fn the_app_is_found_from_the_executable() {
        let app = |p: &str| app_path(Path::new(p));
        assert_eq!(app("/Applications/contr-rune.app/Contents/MacOS/contr-rune"), Some(PathBuf::from("/Applications/contr-rune.app")));
        assert_eq!(app("/Users/me/Applications/My App.app/Contents/MacOS/x"), Some(PathBuf::from("/Users/me/Applications/My App.app")));
        assert_eq!(app("/repo/target/debug/contr-rune"), None);
        assert_eq!(app("/Applications/contr-rune.app/Contents/Other/contr-rune"), None);
        assert_eq!(app("contr-rune"), None);
    }

    #[test]
    fn assets_come_from_the_release_or_the_fixture_folder() {
        assert_eq!(asset_base(None, "v0.2.0"), "https://github.com/contursofran/contr-rune/releases/download/v0.2.0");
        assert_eq!(asset_base(Some("file:///fx/update/newer.json"), "v9.9.9"), "file:///fx/update");
    }

    #[test]
    fn the_helper_gets_the_pid_and_the_paths_and_parses() {
        let args = helper_args(42, Path::new("/tmp/w/unzipped/contr-rune.app"), Path::new("/Applications/My App.app"), Path::new("/tmp/w"));
        assert_eq!(&args[2..], ["sh", "42", "/tmp/w/unzipped/contr-rune.app", "/Applications/My App.app", "/tmp/w"].map(OsString::from));
        // The script must be valid shell: `sh -n` parses it without running it.
        let ok = Command::new("/bin/sh").arg("-n").arg("-c").arg(&args[1]).status().unwrap().success();
        assert!(ok);
        assert!(HELPER.contains("com.apple.quarantine") && HELPER.contains("/usr/bin/open"));
    }

    #[test]
    fn a_toast_comes_once_per_version() {
        let mut u = Updater::new();
        let found = |tag: &str| Some(Newest { tag: tag.into(), items: vec![] });
        u.accept(found("v9.9.9"));
        assert!(u.toast && u.ready.is_some());
        u.toast = false;
        u.accept(found("v9.9.9"));
        assert!(!u.toast, "the same version does not announce again");
        u.accept(found("v10.0.0"));
        assert!(u.toast, "a newer one does");
        u.accept(found("v0.0.1"));
        assert!(u.ready.is_none());
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} up to date \u{b7} checked just now"));
        u.accept(None);
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} could not check"), "a failed check says so");
    }

    #[test]
    fn settings_says_what_the_check_does() {
        let mut u = Updater::new();
        assert_eq!(u.status(), CURRENT, "no check yet");
        u.inflight = true;
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} checking\u{2026}"));
        u.accept(Some(Newest { tag: "v0.0.1".into(), items: vec![] }));
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} up to date \u{b7} checked just now"));
        for (secs, want) in [(59, "just now"), (60, "1 min ago"), (59 * 60, "59 min ago"), (3 * 3600 + 5, "3 h ago")] {
            assert_eq!(ago(Duration::from_secs(secs)), want);
        }
    }
}
