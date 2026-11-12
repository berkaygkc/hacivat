//! Resident set size of this process (F18; no `sysinfo`). The bin's `--smoke` reports it as
//! `idle-rss-mib` against the 200 MB idle budget.

#[cfg(any(target_os = "linux", test))]
const KIB_PER_MIB: u64 = 1024;
#[cfg(any(target_os = "macos", windows))]
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// This process's resident set in MiB (rounded down), or `None` if the OS refuses to say.
///
/// - macOS: `mach_task_basic_info.resident_size` (`task_info(MACH_TASK_BASIC_INFO)`).
/// - Linux: `VmRSS` from `/proc/self/status`.
/// - Windows: `GetProcessMemoryInfo().WorkingSetSize`.
pub fn resident_set_mib() -> Option<u64> {
    imp::resident_set_mib()
}

/// `VmRSS` in MiB from the text of `/proc/self/status` (the kernel reports it in kB).
#[cfg(any(target_os = "linux", test))]
fn vm_rss_mib(status: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let mut fields = line["VmRSS:".len()..].split_whitespace();
    let kib: u64 = fields.next()?.parse().ok()?;
    (fields.next()? == "kB").then_some(kib / KIB_PER_MIB)
}

#[cfg(target_os = "macos")]
mod imp {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::message::mach_msg_type_number_t;
    use mach2::task::task_info;
    use mach2::task_info::{
        MACH_TASK_BASIC_INFO, MACH_TASK_BASIC_INFO_COUNT, mach_task_basic_info,
    };
    use mach2::traps::mach_task_self;
    use mach2::vm_types::integer_t;

    pub(super) fn resident_set_mib() -> Option<u64> {
        let mut info = mach_task_basic_info::default();
        let mut count: mach_msg_type_number_t = MACH_TASK_BASIC_INFO_COUNT;
        // SAFETY: `info` is a properly sized, writable `mach_task_basic_info` and `count` holds
        // its size in `natural_t` units, as `task_info(MACH_TASK_BASIC_INFO)` requires;
        // `mach_task_self()` reads the task's own send right, which is always valid.
        let kr = unsafe {
            task_info(
                mach_task_self(),
                MACH_TASK_BASIC_INFO,
                (&raw mut info).cast::<integer_t>(),
                &mut count,
            )
        };
        if kr != KERN_SUCCESS {
            return None;
        }
        let resident = info.resident_size;
        Some(resident / super::BYTES_PER_MIB)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub(super) fn resident_set_mib() -> Option<u64> {
        super::vm_rss_mib(&std::fs::read_to_string("/proc/self/status").ok()?)
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub(super) fn resident_set_mib() -> Option<u64> {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let cb = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no closing; `counters`
        // is a writable `PROCESS_MEMORY_COUNTERS` of exactly `cb` bytes.
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, cb) };
        if ok == 0 {
            return None;
        }
        Some(counters.WorkingSetSize as u64 / super::BYTES_PER_MIB)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    pub(super) fn resident_set_mib() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::vm_rss_mib;
    use crate::platform;

    #[test]
    fn resident_set_is_reported() {
        let m = platform::resident_set_mib().expect("RSS readable on every OS");
        assert!((1..4096).contains(&m), "{m}");
        let big = vec![1u8; 64 << 20];
        assert!(
            platform::resident_set_mib().unwrap() >= m + 32,
            "64 MiB touched"
        );
        drop(big);
    }

    #[test]
    fn vm_rss_is_parsed_from_proc_status() {
        let status = "Name:\thacivat\nVmPeak:\t  900000 kB\nVmRSS:\t  153600 kB\nThreads:\t12\n";
        assert_eq!(vm_rss_mib(status), Some(150));
        assert_eq!(vm_rss_mib("Name:\thacivat\n"), None);
        assert_eq!(vm_rss_mib("VmRSS:\t  garbage kB\n"), None);
        assert_eq!(vm_rss_mib("VmRSS:\t  2048 MB\n"), None);
    }
}
