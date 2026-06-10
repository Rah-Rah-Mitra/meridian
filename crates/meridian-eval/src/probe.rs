//! Resource probes: process RSS, filesystem free space, Pi temperature/throttle.
//! All best-effort — a probe failure degrades the report, never the bench.
//!
//! The only unsafe in the workspace so far: three libc calls (sysconf, statvfs)
//! with no pointer lifetime subtleties — each wrapped, checked, and contained here.
#![allow(unsafe_code)]

use std::path::Path;

/// Current process resident set size in bytes (from /proc/self/statm).
pub fn rss_bytes() -> u64 {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|f| f.parse::<u64>().ok())
        })
        .map(|pages| pages * page)
        .unwrap_or(0)
}

/// Free bytes on the filesystem containing `path` (statvfs f_bavail × f_frsize).
// statvfs field widths differ across targets (u32 on 32-bit); the conversion is
// identity on our 64-bit targets but required for portability.
#[allow(clippy::useless_conversion)]
pub fn free_disk_bytes(path: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return 0;
    };
    let mut sv: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(cpath.as_ptr(), &raw mut sv) };
    if rc == 0 {
        // Fields are u32 on some targets, u64 on others — normalize via into().
        let bavail: u64 = sv.f_bavail.into();
        let frsize: u64 = sv.f_frsize.into();
        bavail * frsize
    } else {
        0
    }
}

/// SoC temperature in °C via `vcgencmd measure_temp` (Pi-specific; None elsewhere).
pub fn soc_temp_c() -> Option<f64> {
    let out = std::process::Command::new("vcgencmd")
        .arg("measure_temp")
        .output()
        .ok()?;
    // Format: temp=51.0'C
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .strip_prefix("temp=")?
        .trim_end_matches("'C")
        .parse()
        .ok()
}

/// Raw throttle bitmask via `vcgencmd get_throttled` (e.g. 0x0; None off-Pi).
pub fn throttled_flags() -> Option<u32> {
    let out = std::process::Command::new("vcgencmd")
        .arg("get_throttled")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let hex = s.trim().strip_prefix("throttled=0x")?;
    u32::from_str_radix(hex, 16).ok()
}

/// Kernel page size — every report records it (ADR-01: 16K is the supported config).
pub fn page_size() -> u64 {
    (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }) as u64
}
