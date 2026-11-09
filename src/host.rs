//! This Mac's CPU and memory for the "this Mac" server row (A5). The formula is as in the Go
//! contr (D11): CPU is the change in CPU ticks between two readings; used memory is the total
//! minus free, inactive and purgeable pages. Readings run only while the window has the focus.

use std::time::{Duration, Instant};

use rune::Tx;

/// Steady gap between readings while focused (as the Go app).
const EVERY: Duration = Duration::from_secs(2);
/// Gap between the baseline reading taken on focus and the first one that counts: CPU is the
/// change since the previous reading, so the first one after a pause must not cover the pause.
const RESUME_GAP: Duration = Duration::from_millis(250);

/// CPU ticks since boot: busy (user, system, nice) and all (busy plus idle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ticks {
    pub busy: u32,
    pub all: u32,
}

/// One raw reading of the OS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Raw {
    pub ticks: Ticks,
    pub mem_used: u64,
    pub mem_total: u64,
}

/// Whole percents shown in the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub cpu: u32,
    pub mem: u32,
}

/// CPU percent between two tick counts (the counters wrap). `None` when no ticks passed.
pub fn cpu_percent(prev: Ticks, now: Ticks) -> Option<u32> {
    let all = now.all.wrapping_sub(prev.all);
    let busy = now.busy.wrapping_sub(prev.busy);
    (all > 0).then(|| ((f64::from(busy) / f64::from(all) * 100.0).round() as u32).min(100))
}

/// Used memory: total minus (free + inactive + purgeable) pages.
pub fn mem_used(total: u64, page: u64, free: u64, inactive: u64, purgeable: u64) -> u64 {
    total.saturating_sub((free + inactive + purgeable) * page)
}

/// Used share of the total as a whole percent; 0 when the total is unknown.
pub fn mem_percent(used: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((used as f64 / total as f64 * 100.0).round() as u32).min(100)
}

/// The row's meta text: `local`, then the meters when there is a reading.
pub fn meta(r: Option<Reading>) -> String {
    match r {
        Some(r) => format!("local · {}% CPU · {}% MEM", r.cpu, r.mem),
        None => "local".to_owned(),
    }
}

/// The latest reading and the schedule that refreshes it.
pub struct Host {
    pub reading: Option<Reading>,
    prev: Option<Ticks>,
    last: Option<Instant>,
    wait: Duration,
    /// A headless run reads no live numbers: it shows `CONTR_HOST_FAKE` (`cpu,mem`) or nothing.
    fixed: bool,
}

impl Host {
    pub fn new() -> Host {
        Host { reading: None, prev: None, last: None, wait: Duration::ZERO, fixed: false }
    }

    /// Headless: take the numbers of `CONTR_HOST_FAKE=12,48`, never the machine's, and never sample.
    pub fn fix_from_env(&mut self) {
        self.fixed = true;
        self.reading = std::env::var("CONTR_HOST_FAKE").ok().and_then(|v| parse_fake(&v));
    }

    /// Take a reading when one is due and the window has the focus; nothing runs otherwise.
    /// Returns whether the shown numbers changed.
    pub fn poll(&mut self, focused: bool, tx: &mut Tx) -> bool {
        if self.fixed {
            return false;
        }
        if !focused {
            // The next focus starts with a new baseline.
            self.prev = None;
            self.last = None;
            return false;
        }
        if let Some(t) = self.last
            && t.elapsed() < self.wait
        {
            tx.wake_after(self.wait - t.elapsed());
            return false;
        }
        let raw = read_os();
        let had_prev = self.prev.is_some();
        let before = self.reading;
        if let Some(raw) = raw {
            if let Some(cpu) = self.prev.and_then(|p| cpu_percent(p, raw.ticks)) {
                self.reading = Some(Reading { cpu, mem: mem_percent(raw.mem_used, raw.mem_total) });
            }
            self.prev = Some(raw.ticks);
        }
        self.last = Some(Instant::now());
        self.wait = if had_prev || raw.is_none() { EVERY } else { RESUME_GAP };
        tx.wake_after(self.wait);
        self.reading != before
    }
}

fn parse_fake(v: &str) -> Option<Reading> {
    let (cpu, mem) = v.split_once(',')?;
    Some(Reading { cpu: cpu.trim().parse().ok()?, mem: mem.trim().parse().ok()? })
}

unsafe extern "C" {
    // `libc` marks this one deprecated in favour of `mach2`; one declaration is lighter than a crate.
    #[cfg(target_os = "macos")]
    fn mach_host_self() -> libc::host_t;
}

/// One reading of this machine: `host_statistics` for CPU ticks, `host_statistics64` for pages,
/// `hw.memsize` for the total.
#[cfg(target_os = "macos")]
pub fn read_os() -> Option<Raw> {
    use std::mem::size_of;
    // SAFETY: both calls fill a zeroed struct of the size the flavor's count names; the host port
    // is the process's own and needs no release.
    unsafe {
        let host = mach_host_self();
        let mut cpu: libc::host_cpu_load_info = std::mem::zeroed();
        let mut n = libc::HOST_CPU_LOAD_INFO_COUNT;
        if libc::host_statistics(host, libc::HOST_CPU_LOAD_INFO, (&raw mut cpu).cast(), &mut n) != 0 {
            return None;
        }
        let mut vm: libc::vm_statistics64 = std::mem::zeroed();
        let mut n = (size_of::<libc::vm_statistics64>() / size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
        if libc::host_statistics64(host, libc::HOST_VM_INFO64, (&raw mut vm).cast(), &mut n) != 0 {
            return None;
        }
        let mut total: u64 = 0;
        let mut len = size_of::<u64>();
        if libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut total).cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        let page = u64::try_from(libc::sysconf(libc::_SC_PAGESIZE)).ok()?;
        let t = cpu.cpu_ticks;
        let tick = |i: libc::c_int| t[i as usize];
        let busy = tick(libc::CPU_STATE_USER).wrapping_add(tick(libc::CPU_STATE_SYSTEM)).wrapping_add(tick(libc::CPU_STATE_NICE));
        Some(Raw {
            ticks: Ticks { busy, all: busy.wrapping_add(tick(libc::CPU_STATE_IDLE)) },
            mem_used: mem_used(total, page, vm.free_count.into(), vm.inactive_count.into(), vm.purgeable_count.into()),
            mem_total: total,
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn read_os() -> Option<Raw> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(busy: u32, all: u32) -> Ticks {
        Ticks { busy, all }
    }

    #[test]
    fn cpu_is_the_busy_share_of_the_ticks_between_two_readings() {
        let cases = [
            (t(0, 0), t(12, 100), Some(12)),
            (t(100, 400), t(150, 500), Some(50)),
            (t(10, 100), t(10, 100), None),
            (t(0, 0), t(1, 3), Some(33)),
            (t(0, 0), t(2, 3), Some(67)),
            (t(u32::MAX - 4, u32::MAX - 9), t(5, 10), Some(50)),
            (t(0, 0), t(300, 100), Some(100)),
        ];
        for (a, b, want) in cases {
            assert_eq!(cpu_percent(a, b), want, "{a:?} -> {b:?}");
        }
    }

    #[test]
    fn memory_is_total_minus_free_inactive_and_purgeable_pages() {
        // 16 GiB, 16 KiB pages: 100k free + 200k inactive + 50k purgeable.
        let total = 16u64 << 30;
        let used = mem_used(total, 16384, 100_000, 200_000, 50_000);
        assert_eq!(used, total - 350_000 * 16384);
        assert_eq!(mem_percent(used, total), 67);
        assert_eq!(mem_used(100, 10, 20, 0, 0), 0);
        assert_eq!(mem_percent(5, 0), 0);
        assert_eq!(mem_percent(200, 100), 100);
    }

    #[test]
    fn meta_shows_the_meters_once_there_is_a_reading() {
        assert_eq!(meta(None), "local");
        assert_eq!(meta(Some(Reading { cpu: 12, mem: 48 })), "local · 12% CPU · 48% MEM");
    }

    #[test]
    fn fake_numbers_parse_and_bad_ones_do_not() {
        assert_eq!(parse_fake("12,48"), Some(Reading { cpu: 12, mem: 48 }));
        assert_eq!(parse_fake(" 7 , 9 "), Some(Reading { cpu: 7, mem: 9 }));
        assert_eq!(parse_fake("12"), None);
        assert_eq!(parse_fake("a,b"), None);
    }
}
