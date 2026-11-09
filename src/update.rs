//! The update check (R3a): ask GitHub for the latest release with `/usr/bin/curl` on a thread, and
//! keep the newer version (if any) for the title-bar pill, the toast and the Settings row.
//! `CONTR_UPDATE_URL` points the check at another URL (a `file://` JSON in tests); a headless run
//! without it makes no check and never touches the network.

use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use rune::{El, Tx, Waker, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Placement, Toast, ToastRegion, popover};

use crate::App;

const LATEST_URL: &str = "https://api.github.com/repos/contursofran/contr-rune/releases/latest";
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

/// The update state of the app.
pub struct Updater {
    /// The newer release found by the last good check.
    pub ready: Option<Release>,
    /// Some check has succeeded (Settings says "up to date" only then).
    checked: bool,
    last_try: Option<Instant>,
    last_ok: Option<Instant>,
    inflight: bool,
    /// A check was asked for (focus, the Settings button) and waits for `poll`.
    wanted: bool,
    /// The version the toast was shown for: one toast per version.
    toasted: Option<String>,
    pub toast: bool,
    pub panel: bool,
    /// "Restart now" was chosen. R3b downloads and swaps the app from here.
    #[allow(dead_code)]
    restart_wanted: bool,
    tx: Sender<Found>,
    rx: Receiver<Found>,
}

impl Updater {
    pub fn new() -> Updater {
        let (tx, rx) = channel();
        Updater {
            ready: None,
            checked: false,
            last_try: None,
            last_ok: None,
            inflight: false,
            wanted: false,
            toasted: None,
            toast: false,
            panel: false,
            restart_wanted: false,
            tx,
            rx,
        }
    }

    /// Take a result in (and show the toast for a version not yet announced). Returns whether the view changes.
    fn accept(&mut self, found: Found) -> bool {
        self.inflight = false;
        let Some(newest) = found else { return false };
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
        let Some(url) = url() else { return changed };
        let due = self.last_try.is_none_or(|t| t.elapsed() >= EVERY);
        if !self.inflight && (self.wanted || (focused && due)) {
            self.wanted = false;
            self.inflight = true;
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

    /// "Restart now": the seam R3b fills (download, verify, swap, relaunch).
    pub fn restart(&mut self) {
        self.restart_wanted = true;
        self.toast = false;
        self.panel = false;
    }

    /// The Settings value after the app's version.
    pub fn status(&self) -> String {
        match (&self.ready, self.checked) {
            (Some(r), _) => format!("{CURRENT} \u{b7} {} available", r.version),
            (None, true) => format!("{CURRENT} \u{b7} up to date"),
            (None, false) => CURRENT.to_owned(),
        }
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
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} up to date"));
        u.accept(None);
        assert_eq!(u.status(), format!("{CURRENT} \u{b7} up to date"), "a failed check keeps the last answer");
    }
}
