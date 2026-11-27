// ## m4b-cifix-5-windows-identity
//! A file's or folder's identity on Windows (CI 45091594060).
//!
//! In a file of its own: `xtask check-deps`' `BackendInPublicApi` reads a `pub const` holding
//! `<<` (as `fileio.rs`' `STREAM_CHUNK_BYTES`) as a header open to the end of its file, so a
//! `file_id` path anywhere after it reads as public there.

use super::FileIdentity;

/// The identity of the file or folder at `path` on Windows: the volume serial number and the
/// file id (`GetFileInformationByHandleEx` `FileIdInfo`, else `GetFileInformationByHandle`'s
/// file index), read through `notify-debouncer-full`'s re-export of `file-id`, so there is no
/// `unsafe` here and no new dependency. `None` when it cannot be read.
///
/// CI 45091594060: without it a Windows library root was compared by path alone, so a root
/// moved away and replaced by another folder (a restore) was served as the same library, and a
/// file renamed over a script mid-run went unseen when its size and times matched (NTFS
/// tunnelling hands the new name the old creation time). On NTFS the file id is the file
/// reference number, whose sequence number changes when a record is reused, so a folder or
/// file made again at the same path gets another id; a rename keeps it.
///
/// No birth time is part of it (R-CI2-1): userland sets a Windows creation time
/// (`SetFileTime`, `robocopy /DCOPY:T`), and tunnelling gives a name made again within 15 s
/// its old creation time, so it would refuse a restored root without telling a replaced one
/// apart. Residuals (ADR 0009 `m4b-cifix-5`): FAT and exFAT derive the file index from the
/// folder entry's place (a rename changes it, so the library is refused and offered "Use this
/// folder"; an entry made again in the same slot keeps it), and some network shares report no
/// stable id.
#[cfg(windows)]
pub(super) fn windows_identity(path: &std::path::Path) -> Option<FileIdentity> {
    use notify_debouncer_full::file_id as backend;
    match backend::get_file_id(path).ok()? {
        backend::FileId::HighRes {
            volume_serial_number,
            file_id,
        } => identity_from(volume_serial_number, file_id),
        backend::FileId::LowRes {
            volume_serial_number,
            file_index,
        } => identity_from(u64::from(volume_serial_number), u128::from(file_index)),
        backend::FileId::Inode { .. } => None,
    }
}

/// The identity a volume serial and a raw file id give; `None` for the file id 0.
///
/// ## m4b-cifix-5 review m1: WebDAV and some NAS or FUSE shares answer 0 for every file. Two
/// zeros never prove the same folder, so a zero id is no identity at all: the root is then
/// compared by its path, as where none can be read. Residual (ADR 0009): a share whose ids
/// change from one open to the next refuses an unchanged root (fail closed).
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "Windows only; tested everywhere")
)]
pub(super) fn identity_from(dev: u64, file_id: u128) -> Option<FileIdentity> {
    (file_id != 0).then(|| FileIdentity {
        dev,
        ino: fold_file_id(file_id),
        born: None,
    })
}

/// A 128-bit file id as the 64-bit `ino`: NTFS's is its 64-bit file reference number
/// zero-extended, kept exactly; a ReFS id that uses the high half is hashed (XXH3-64 of its
/// bytes), so two ids collide only by chance.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "Windows only; tested everywhere")
)]
pub(super) fn fold_file_id(id: u128) -> u64 {
    match u64::try_from(id) {
        Ok(low) => low,
        Err(_) => twox_hash::XxHash3_64::oneshot(&id.to_le_bytes()),
    }
}
// ## m4b-cifix-5-windows-identity
