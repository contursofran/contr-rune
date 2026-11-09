//! Opening a line to a core: `hc connect` as a child process, or the in-process fake (Contract
//! C9). Finding the program follows C1/§1.2: `CONTR_HC`, then `PATH`, then the usual folders (an
//! app opened from Finder has a short `PATH`).

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::fake::{self, Fixture};
use super::watch::HcStatus;

/// How to reach a core.
#[derive(Clone, Debug)]
pub enum Dialer {
    /// Spawn `<path> connect`.
    Command { path: PathBuf },
    /// An in-process fake core.
    Fake(Arc<Fixture>),
}

/// Keeps the child alive with the line and kills it when dropped.
pub struct Keep {
    stderr: Arc<Mutex<String>>,
    child: Option<Child>,
}

impl Keep {
    /// The last line `hc connect` wrote to stderr, if any.
    pub fn stderr_tail(&self) -> Option<String> {
        let s = self.stderr.lock().unwrap();
        s.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned())
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub struct Link {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub keep: Keep,
}

impl Dialer {
    /// Choose from the environment. `Err` is the status to show when there is nothing to dial:
    /// `CONTR_HC=off` is `Off`; a headless run without `CONTR_HC_FAKE`, or no program found, is
    /// `NotInstalled`; a broken fixture is `Failed`.
    pub fn from_env(headless: bool) -> Result<Dialer, HcStatus> {
        let var = |k: &str| std::env::var_os(k);
        let home = var("HOME").map(PathBuf::from);
        Dialer::choose(var("CONTR_HC_FAKE").as_deref(), var("CONTR_HC").as_deref(), headless, var("PATH").as_deref(), home.as_deref())
    }

    pub fn choose(fake: Option<&OsStr>, hc: Option<&OsStr>, headless: bool, path: Option<&OsStr>, home: Option<&Path>) -> Result<Dialer, HcStatus> {
        if let Some(f) = fake.filter(|f| !f.is_empty()) {
            return Fixture::load(Path::new(f)).map(|fx| Dialer::Fake(Arc::new(if headless { fx.with_manual_clock() } else { fx }))).map_err(|e| HcStatus::Failed(e.to_string()));
        }
        if hc == Some(OsStr::new("off")) {
            return Err(HcStatus::Off);
        }
        if headless {
            return Err(HcStatus::NotInstalled);
        }
        find_hc(hc, path, home).map(|path| Dialer::Command { path }).ok_or(HcStatus::NotInstalled)
    }

    /// The first and the longest wait before a tab attaches again after a lost line (Contract C2:
    /// 0.5 s doubled to 5 s). The in-process fake is short, so headless tests need not wait.
    pub fn backoff(&self) -> (Duration, Duration) {
        match self {
            Dialer::Command { .. } => (Duration::from_millis(500), Duration::from_secs(5)),
            Dialer::Fake(_) => (Duration::from_millis(5), Duration::from_millis(20)),
        }
    }

    /// How long a tab told `unavailable` waits for `restarted` before it tries again anyway (the
    /// event may have been missed).
    pub fn safety(&self) -> Duration {
        match self {
            Dialer::Command { .. } => Duration::from_secs(10),
            Dialer::Fake(_) => Duration::from_millis(30),
        }
    }

    /// Headless runs only: move the fake core's clock to `secs` of script time (so the fixture's
    /// `after_ms` follows the script) and wait for the work that events started. A real hc has no
    /// clock to move.
    pub fn step_fake(&self, secs: f64) {
        if let Dialer::Fake(fx) = self {
            fx.set_time((secs * 1000.0) as u64);
        }
    }

    /// Headless runs only: tell the fake core that the app took an update from it.
    pub fn note_fake_activity(&self) {
        if let Dialer::Fake(fx) = self {
            fx.bump();
        }
    }

    /// Headless runs only: a tab's dial-run-attach steps hold the script until they end.
    pub fn work_fake(&self) -> Option<fake::Work> {
        match self {
            Dialer::Fake(fx) => Some(fx.work()),
            Dialer::Command { .. } => None,
        }
    }

    /// Headless runs only: hold the script until the fake core has been quiet for a moment.
    pub fn settle_fake(&self) {
        if let Dialer::Fake(fx) = self {
            fx.settle();
        }
    }

    /// How often an attached stream looks for a `restarted` event or a closed tab.
    pub fn poll(&self) -> Duration {
        match self {
            Dialer::Command { .. } => Duration::from_millis(250),
            Dialer::Fake(_) => Duration::from_millis(5),
        }
    }

    /// Open one new line to the core.
    pub fn dial(&self) -> io::Result<Link> {
        match self {
            Dialer::Command { path } => {
                let mut child = Command::new(path).arg("connect").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
                let (stdin, stdout, stderr) = (child.stdin.take().unwrap(), child.stdout.take().unwrap(), child.stderr.take().unwrap());
                let tail = Arc::new(Mutex::new(String::new()));
                let t = tail.clone();
                let _ = std::thread::Builder::new().name("hc-stderr".into()).spawn(move || {
                    let mut stderr = stderr;
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = stderr.read(&mut buf) {
                        if n == 0 {
                            return;
                        }
                        let mut s = t.lock().unwrap();
                        s.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if s.len() > 2048 {
                            let mut cut = s.len() - 2048;
                            while !s.is_char_boundary(cut) {
                                cut += 1;
                            }
                            s.drain(..cut);
                        }
                    }
                });
                Ok(Link { reader: Box::new(stdout), writer: Box::new(stdin), keep: Keep { stderr: tail, child: Some(child) } })
            }
            Dialer::Fake(fx) => {
                let (req_r, req_w) = fake::pipe();
                let (rep_r, rep_w) = fake::pipe();
                let fx = fx.clone();
                std::thread::Builder::new().name("hc-fake".into()).spawn(move || fake::serve(fx, Box::new(req_r), Box::new(rep_w)))?;
                Ok(Link { reader: Box::new(rep_r), writer: Box::new(req_w), keep: Keep { stderr: Arc::default(), child: None } })
            }
        }
    }
}

fn is_program(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Find the hc program. An explicit `CONTR_HC` (non-empty) is the only candidate; otherwise `hc`
/// on `PATH`, then `~/.local/bin/hc`, `~/.local/bin/herdr-core`, `/opt/homebrew/bin/hc`,
/// `/usr/local/bin/hc`.
pub fn find_hc(contr_hc: Option<&OsStr>, path: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = contr_hc.filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return is_program(&p).then_some(p);
    }
    let on_path = path.into_iter().flat_map(std::env::split_paths).map(|d| d.join("hc"));
    let usual = home.into_iter().flat_map(|h| [h.join(".local/bin/hc"), h.join(".local/bin/herdr-core")]).chain(["/opt/homebrew/bin/hc", "/usr/local/bin/hc"].map(PathBuf::from));
    on_path.chain(usual).find(|p| is_program(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("contr-rune-dial-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn program(dir: &Path, rel: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn contr_hc_wins_over_path_and_home() {
        let d = scratch();
        let (own, on_path, _home_hc) = (program(&d, "own/hc2"), program(&d, "bin/hc"), program(&d, "home/.local/bin/hc"));
        let found = find_hc(Some(own.as_os_str()), Some(d.join("bin").as_os_str()), Some(&d.join("home")));
        assert_eq!(found, Some(own));
        assert_ne!(found, Some(on_path));
    }

    #[test]
    fn a_missing_contr_hc_is_not_replaced_by_a_search() {
        let d = scratch();
        program(&d, "bin/hc");
        assert_eq!(find_hc(Some(d.join("nope").as_os_str()), Some(d.join("bin").as_os_str()), None), None);
        // An empty value counts as unset.
        assert_eq!(find_hc(Some(OsStr::new("")), Some(d.join("bin").as_os_str()), None), Some(d.join("bin/hc")));
    }

    #[test]
    fn path_comes_before_the_home_folders() {
        let d = scratch();
        let on_path = program(&d, "bin/hc");
        program(&d, "home/.local/bin/hc");
        assert_eq!(find_hc(None, Some(d.join("bin").as_os_str()), Some(&d.join("home"))), Some(on_path));
    }

    #[test]
    fn home_local_bin_hc_then_herdr_core() {
        let d = scratch();
        let core = program(&d, "home/.local/bin/herdr-core");
        assert_eq!(find_hc(None, None, Some(&d.join("home"))), Some(core));
        let hc = program(&d, "home/.local/bin/hc");
        assert_eq!(find_hc(None, None, Some(&d.join("home"))), Some(hc));
    }

    #[test]
    fn a_file_that_is_not_executable_is_skipped() {
        let d = scratch();
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/hc"), "x").unwrap();
        #[cfg(unix)]
        assert_eq!(find_hc(None, Some(d.join("bin").as_os_str()), Some(&d.join("empty"))).filter(|p| p.starts_with(&d)), None);
    }

    #[test]
    fn choose_off_headless_missing_and_fake() {
        let d = scratch();
        let fixture = d.join("f.json");
        std::fs::write(&fixture, "{}").unwrap();
        let hc = program(&d, "bin/hc");
        let path = d.join("bin");
        let pick = |fake: Option<&OsStr>, hc: Option<&OsStr>, headless| Dialer::choose(fake, hc, headless, Some(path.as_os_str()), None);
        assert!(matches!(pick(Some(fixture.as_os_str()), None, true), Ok(Dialer::Fake(_))), "the fake works headless");
        assert!(matches!(pick(Some(fixture.as_os_str()), Some(OsStr::new("off")), false), Ok(Dialer::Fake(_))), "the fake wins over off");
        assert!(matches!(pick(None, Some(OsStr::new("off")), false), Err(HcStatus::Off)));
        assert!(matches!(pick(None, None, true), Err(HcStatus::NotInstalled)), "headless never dials a real hc");
        assert!(matches!(pick(None, Some(hc.as_os_str()), true), Err(HcStatus::NotInstalled)), "not even with CONTR_HC");
        assert!(matches!(pick(None, None, false), Ok(Dialer::Command { .. })));
        let bad = pick(Some(d.join("missing.json").as_os_str()), None, false);
        assert!(matches!(bad, Err(HcStatus::Failed(m)) if m.contains("missing.json")));
    }
}
