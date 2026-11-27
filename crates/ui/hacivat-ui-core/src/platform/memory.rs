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
    let rest = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let mut fields = rest.split_whitespace();
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

// ## m3a-t01b-seams
/// The machine's physical memory in bytes, or `None` when the OS does not say: the result
/// store's global budget is a quarter of it, at most 4 GiB (R-M3-19).
///
/// - macOS: `sysctlbyname("hw.memsize")` through `libc`.
/// - Windows: `GlobalMemoryStatusEx().ullTotalPhys` (`Win32_System_SystemInformation`).
/// - Linux: `MemTotal` of `/proc/meminfo` (the kernel reports it in kB).
///
/// The dependencies are Task 1b's rows; the bodies are Task 33's.
pub fn physical_memory_bytes() -> Option<u64> {
    // ## m3a-t33-exec-ui
    phys::physical_memory_bytes().filter(|b| *b > 0)
}

// ## m3a-t33-exec-ui
/// `MemTotal` in bytes from the text of `/proc/meminfo` (the kernel reports it in kB).
#[cfg(any(target_os = "linux", test))]
fn mem_total_bytes(meminfo: &str) -> Option<u64> {
    let rest = meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
    let mut fields = rest.split_whitespace();
    let kib: u64 = fields.next()?.parse().ok()?;
    if fields.next()? != "kB" {
        return None;
    }
    kib.checked_mul(1024)
}

#[cfg(target_os = "macos")]
mod phys {
    pub(super) fn physical_memory_bytes() -> Option<u64> {
        let mut bytes: u64 = 0;
        let mut len: libc::size_t = size_of::<u64>();
        // SAFETY: the name is a NUL-terminated C string literal; `bytes` is a writable `u64`
        // and `len` holds its size, which `sysctlbyname` reads as the output buffer's length
        // and overwrites with the length it wrote; no new value is set (null pointer, 0).
        let rc = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut bytes).cast::<libc::c_void>(),
                &raw mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && len == size_of::<u64>()).then_some(bytes)
    }
}

#[cfg(target_os = "linux")]
mod phys {
    pub(super) fn physical_memory_bytes() -> Option<u64> {
        super::mem_total_bytes(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

#[cfg(windows)]
mod phys {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub(super) fn physical_memory_bytes() -> Option<u64> {
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(size_of::<MEMORYSTATUSEX>()).ok()?,
            ..Default::default()
        };
        // SAFETY: `status` is a writable `MEMORYSTATUSEX` whose `dwLength` holds its size, as
        // `GlobalMemoryStatusEx` requires.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        (ok != 0).then_some(status.ullTotalPhys)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod phys {
    pub(super) fn physical_memory_bytes() -> Option<u64> {
        None
    }
}

// ## demofix-1 fix round 1 (I2): the F6 measurement's sampler.
/// This process's physical footprint in bytes, what Activity Monitor's "Memory" column shows
/// (macOS `task_vm_info.phys_footprint`: dirty and compressed memory, the GPU's included,
/// clean file-backed pages not); `None` on other systems.
pub fn phys_footprint_bytes() -> Option<u64> {
    footprint::phys_footprint_bytes()
}

#[cfg(target_os = "macos")]
mod footprint {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::message::mach_msg_type_number_t;
    use mach2::task::task_info;
    use mach2::task_info::{TASK_VM_INFO, task_vm_info};
    use mach2::traps::mach_task_self;
    use mach2::vm_types::{integer_t, natural_t};

    pub(super) fn phys_footprint_bytes() -> Option<u64> {
        let mut info = task_vm_info::default();
        let mut count = (std::mem::size_of::<task_vm_info>() / std::mem::size_of::<natural_t>())
            as mach_msg_type_number_t;
        // SAFETY: `info` is a properly sized, writable `task_vm_info` and `count` holds its size
        // in `natural_t` units, as `task_info(TASK_VM_INFO)` requires (the kernel fills at most
        // `count` units and lowers `count` to what it filled); `mach_task_self()` reads the
        // task's own send right, which is always valid.
        let kr = unsafe {
            task_info(
                mach_task_self(),
                TASK_VM_INFO,
                (&raw mut info).cast::<integer_t>(),
                &mut count,
            )
        };
        if kr != KERN_SUCCESS {
            return None;
        }
        let footprint = info.phys_footprint;
        Some(footprint)
    }
}

#[cfg(not(target_os = "macos"))]
mod footprint {
    pub(super) fn phys_footprint_bytes() -> Option<u64> {
        None
    }
}
// ## demofix-1

#[cfg(test)]
mod tests {
    use super::vm_rss_mib;
    use crate::platform;

    // ## demofix-1 fix round 1 (I2)
    #[cfg(target_os = "macos")]
    #[test]
    fn phys_footprint_follows_touched_memory() {
        let before = platform::phys_footprint_bytes().expect("readable on macOS");
        assert!(before > 1 << 20, "{before}");
        let big = vec![1u8; 64 << 20];
        let after = platform::phys_footprint_bytes().unwrap();
        assert!(after >= before + (32 << 20), "{before} -> {after}");
        drop(big);
    }

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

    // ## m3a-t33-exec-ui
    #[test]
    fn physical_memory_is_known_on_this_platform() {
        let bytes = platform::physical_memory_bytes().unwrap_or(0);
        assert!(bytes > 1 << 30, "{bytes}");
    }

    #[test]
    fn mem_total_is_parsed_from_proc_meminfo() {
        let meminfo = "MemTotal:       16318976 kB\nMemFree:         1234 kB\n";
        assert_eq!(super::mem_total_bytes(meminfo), Some(16_318_976 * 1024));
        assert_eq!(super::mem_total_bytes("MemFree: 1 kB\n"), None);
        assert_eq!(super::mem_total_bytes("MemTotal: x kB\n"), None);
        assert_eq!(super::mem_total_bytes("MemTotal: 16 MB\n"), None);
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
