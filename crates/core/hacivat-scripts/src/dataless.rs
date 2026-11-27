//! Cloud placeholders from metadata only: macOS `SF_DATALESS`, Windows
//! `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (HV-SCRIPT-03 step 6; R-M4b-15). Task 4 writes it.
//!
//! ## m4b-t01a-seams: declared by Task 1a so later tasks add no `lib.rs` line.
//!
//! ## m4b-t04-scanner
//! A dataless file is listed from its metadata and never read by a scan or hashed by status
//! (Task 7 shows it as `Unknown`). The flags come from the `lstat` the scan already made, so
//! the check opens nothing. On Windows, the two recall attributes and `OFFLINE` all mean "the
//! bytes are not here"; any of them counts (fail closed: a file that is wrongly called
//! dataless only loses its status badge, a file wrongly called local would be downloaded).
//! Other systems have no such flag.

use std::fs::Metadata;

/// `SF_DATALESS` (`0x40000000`, `<sys/stat.h>`); `libc` 0.2.191 does not define it.
const SF_DATALESS: u32 = 1 << 30;

/// `FILE_ATTRIBUTE_OFFLINE` (`0x1000`), `FILE_ATTRIBUTE_RECALL_ON_OPEN` (`0x40000`) and
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (`0x400000`), `winnt.h`.
const WINDOWS_NOT_HERE: u32 = (1 << 12) | (1 << 18) | (1 << 22);

/// Whether macOS `st_flags` mark a dataless file.
pub fn from_st_flags(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

/// Whether Windows file attributes mark a file whose bytes are not on the disk.
pub fn from_file_attributes(attrs: u32) -> bool {
    attrs & WINDOWS_NOT_HERE != 0
}

/// Whether `meta` (from `symlink_metadata`) describes a dataless file.
#[cfg(target_os = "macos")]
pub fn is_dataless(meta: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    from_st_flags(meta.st_flags())
}

/// Whether `meta` (from `symlink_metadata`) describes a dataless file.
#[cfg(windows)]
pub fn is_dataless(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    from_file_attributes(meta.file_attributes())
}

/// Whether `meta` (from `symlink_metadata`) describes a dataless file: never, here.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_dataless(meta: &Metadata) -> bool {
    let _ = meta;
    false
}
// ## m4b-t04-scanner
