//! Process-level allocator backstop for long-lived Sessions. Structural work
//! keeps live state small; glibc can still retain freed arenas, so trim only
//! when RSS is already large and at most once per few seconds.

use std::sync::atomic::{AtomicU64, Ordering};

const TRIM_THRESHOLD_KB: u64 = 40 * 1024;
const TRIM_INTERVAL_MS: u64 = 5_000;

static LAST_TRIM_MS: AtomicU64 = AtomicU64::new(0);

fn rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.trim().trim_end_matches(" kB").parse().ok())
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(target_os = "linux")]
pub(crate) fn trim_if_large() {
    if rss_kb().is_none_or(|rss| rss < TRIM_THRESHOLD_KB) {
        return;
    }
    let now = now_ms();
    let last = LAST_TRIM_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < TRIM_INTERVAL_MS {
        return;
    }
    if LAST_TRIM_MS
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn trim_if_large() {}
