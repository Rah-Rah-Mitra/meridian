//! System probes shared by the shed monitor (here) and the bench harness
//! (meridian-eval re-uses these). Best-effort: a probe failure returns a safe
//! default, never panics.
//!
//! The only unsafe in this crate: two libc calls (sysconf, statvfs) with no
//! pointer-lifetime subtleties.
#![allow(unsafe_code)]

use std::path::Path;

/// Current process resident set size in bytes (/proc/self/statm).
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
// statvfs field widths differ across targets; identity conversion on 64-bit.
#[allow(clippy::useless_conversion)]
pub fn free_disk_bytes(path: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return 0;
    };
    let mut sv: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(cpath.as_ptr(), &raw mut sv) };
    if rc == 0 {
        let bavail: u64 = sv.f_bavail.into();
        let frsize: u64 = sv.f_frsize.into();
        bavail * frsize
    } else {
        0
    }
}

/// SoC temperature in °C. Reads sysfs thermal zones (works inside containers,
/// unlike vcgencmd); returns the hottest zone, or None off-Linux/odd platforms.
pub fn soc_temp_c() -> Option<f64> {
    let mut max: Option<f64> = None;
    for i in 0..4 {
        let path = format!("/sys/class/thermal/thermal_zone{i}/temp");
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(millic) = raw.trim().parse::<f64>() {
                let c = millic / 1000.0;
                if (0.0..150.0).contains(&c) {
                    max = Some(max.map_or(c, |m: f64| m.max(c)));
                }
            }
        }
    }
    max
}

pub fn page_size() -> u64 {
    (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }) as u64
}
