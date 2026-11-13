//! The encrypted-file vault (HV-SEC-05, D-016, B13, Q30). Every vault is a temp fixture with
//! fast Argon2 parameters; secret comparisons never print either value.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hacivat_secrets::{
    ExposeSecret, FileVault, SecretBackend, SecretError, SecretKey, SecretSlot, SecretString,
    VaultBackend, VaultParams,
};

/// Compares a secret without printing either value on failure.
macro_rules! assert_secret {
    ($got:expr, $want:expr) => {
        assert!(
            $got.expect("a stored secret").expose_secret() == $want,
            "secret mismatch (values not shown)"
        )
    };
}

thread_local! {
    /// Keeps each fixture's directory alive until the test thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

const MASTER: &str = "correct horse";

fn fast() -> VaultParams {
    VaultParams {
        m_kib: 64,
        t: 1,
        p: 1,
    }
}

fn sec(s: &str) -> SecretString {
    SecretString::from(s)
}

/// The same key on every call.
fn key() -> SecretKey {
    SecretKey {
        profile: "0b0f5a3c-2f7e-4a54-9d3b-6c1e8f2a7d10"
            .parse()
            .expect("a valid UUID"),
        slot: SecretSlot::DbPassword,
    }
}

fn other_key() -> SecretKey {
    SecretKey {
        slot: SecretSlot::SshPassword { hop: 1 },
        ..key()
    }
}

/// A fresh directory that lives until the test thread ends.
fn tmp_dir() -> PathBuf {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().to_path_buf();
    DIRS.with(|d| d.borrow_mut().push(dir));
    path
}

fn vault_tmp_path() -> (FileVault, PathBuf) {
    let path = tmp_dir().join("vault.bin");
    (FileVault::with_params(path.clone(), fast()), path)
}

fn vault_tmp() -> FileVault {
    vault_tmp_path().0
}

fn created(master: &str) -> FileVault {
    let v = vault_tmp();
    v.create(sec(master)).expect("the vault is created");
    v
}

/// An unlocked vault holding `s3cret` under `key()`.
fn created_with_secret() -> (FileVault, PathBuf) {
    let (v, path) = vault_tmp_path();
    v.create(sec(MASTER)).expect("the vault is created");
    v.put(&key(), &sec("s3cret")).expect("the secret is stored");
    (v, path)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn flip_byte(path: &Path, at: usize) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[at] ^= 1;
    std::fs::write(path, bytes).unwrap();
}

fn truncate(path: &Path, len: usize) {
    let bytes = std::fs::read(path).unwrap();
    std::fs::write(path, &bytes[..len]).unwrap();
}

fn write_u16(path: &Path, at: usize, value: u16) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

/// Rewrites the header's Argon2 parameters only (the body stays sealed under the old header).
fn set_params(path: &Path, m: u32, t: u32, p: u32) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[12..16].copy_from_slice(&m.to_le_bytes());
    bytes[16..20].copy_from_slice(&t.to_le_bytes());
    bytes[20..24].copy_from_slice(&p.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// What a newer Hacivat writes: the same contents sealed under a header with `format_major =
/// major` (the header is the associated data, so the whole body is re-sealed).
fn set_major(path: &Path, major: u16) {
    let bytes = std::fs::read(path).unwrap();
    let (header, body) = bytes.split_at(64);
    let params = Params::new(
        u32_at(header, 12),
        u32_at(header, 16),
        u32_at(header, 20),
        Some(32),
    )
    .unwrap();
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(MASTER.as_bytes(), &header[24..40], &mut key)
        .unwrap();
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let mut plain = body.to_vec();
    cipher
        .decrypt_in_place(XNonce::from_slice(&header[40..64]), header, &mut plain)
        .expect("the fixture opens");
    let mut newer = header.to_vec();
    newer[8..10].copy_from_slice(&major.to_le_bytes());
    cipher
        .encrypt_in_place(XNonce::from_slice(&newer[40..64]), &newer, &mut plain)
        .unwrap();
    newer.extend_from_slice(&plain);
    std::fs::write(path, newer).unwrap();
}

fn nonce_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()[40..64].to_vec()
}

fn salt_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()[24..40].to_vec()
}

fn tmp_of(path: &Path) -> PathBuf {
    path.with_file_name("vault.bin.tmp")
}

#[test]
fn vault_round_trip() {
    let v = vault_tmp();
    v.create(sec(MASTER)).unwrap();
    v.put(&key(), &sec("s3cret")).unwrap();
    v.lock();
    assert!(matches!(v.get(&key()), Err(SecretError::VaultLocked)));
    v.unlock(sec(MASTER)).unwrap();
    assert_secret!(v.get(&key()).unwrap(), "s3cret");
    assert!(v.get(&other_key()).unwrap().is_none());
    v.delete(&key()).unwrap();
    v.delete(&key()).unwrap();
    assert!(v.get(&key()).unwrap().is_none());
}

#[test]
fn vault_file_never_contains_plaintext() {
    // Q5
    let (v, path) = vault_tmp_path();
    v.create(sec(MASTER)).unwrap();
    v.put(&key(), &sec("s3cret-value")).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!contains(&bytes, b"s3cret-value") && !contains(&bytes, key().service().as_bytes()));
    assert!(!contains(&bytes, MASTER.as_bytes()));
    assert!(!tmp_of(&path).exists(), "no temp file is left behind");
    #[cfg(unix)]
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn the_header_follows_the_layout() {
    let (v, path) = vault_tmp_path();
    v.create(sec(MASTER)).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..8], b"HVVAULT1");
    assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 1);
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 0);
    assert_eq!(
        (u32_at(&bytes, 12), u32_at(&bytes, 16), u32_at(&bytes, 20)),
        (64, 1, 1)
    );
    // An empty map `{}` plus the 16-byte tag.
    assert_eq!(bytes.len(), 64 + 2 + 16);
}

#[test]
fn wrong_master_password() {
    let v = created(MASTER);
    v.lock();
    assert_eq!(
        v.unlock(sec("wrong horse")),
        Err(SecretError::WrongMasterPassword)
    );
    assert!(!v.is_unlocked());
}

#[test]
fn tampered_vault_is_untouched() {
    let (v, path) = created_with_secret();
    v.lock();
    flip_byte(&path, 70);
    let before = std::fs::read(&path).unwrap();
    // Q30: the AEAD cannot tell tamper from a wrong password
    assert!(matches!(
        v.unlock(sec(MASTER)),
        Err(SecretError::WrongMasterPassword | SecretError::VaultCorrupt)
    ));
    assert!(v.put(&key(), &sec("x")).is_err());
    assert!(v.delete(&key()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn truncated_vault_is_corrupt() {
    let (v, path) = created_with_secret();
    v.lock();
    truncate(&path, 20);
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::VaultCorrupt));
    let (v, path) = created_with_secret();
    v.lock();
    truncate(&path, 64 + 15);
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::VaultCorrupt));
}

#[test]
fn a_bad_magic_is_corrupt() {
    let (v, path) = created_with_secret();
    v.lock();
    flip_byte(&path, 0);
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::VaultCorrupt));
}

/// The header is the associated data: a header field that does not feed the key (the minor
/// version) still cannot change without the unlock failing.
#[test]
fn the_header_is_authenticated() {
    let (v, path) = created_with_secret();
    v.lock();
    write_u16(&path, 10, 1);
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::WrongMasterPassword));
    assert!(!v.is_unlocked());
}

/// A downgraded cost in the header derives another key, and the header is authenticated:
/// it never decrypts.
#[test]
fn downgraded_header_params_never_decrypt() {
    let path = tmp_dir().join("vault.bin");
    let v = FileVault::with_params(
        path.clone(),
        VaultParams {
            m_kib: 8192,
            t: 2,
            p: 1,
        },
    );
    v.create(sec(MASTER)).unwrap();
    v.put(&key(), &sec("s3cret")).unwrap();
    v.lock();
    set_params(&path, 8192, 1, 1);
    let v = FileVault::open(path);
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::WrongMasterPassword));
}

#[test]
fn newer_vault_version_is_read_only() {
    let (v, path) = created_with_secret();
    v.lock();
    set_major(&path, 2);
    let before = std::fs::read(&path).unwrap();
    let v = FileVault::with_params(path.clone(), fast());
    // Q30: one unlock, Ok
    assert!(v.unlock(sec(MASTER)).is_ok());
    assert_secret!(v.get(&key()).unwrap(), "s3cret");
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultReadOnly));
    assert_eq!(v.delete(&key()), Err(SecretError::VaultReadOnly));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn crafted_header_params_are_corrupt() {
    // Q30
    for (m, t, p) in [
        (u32::MAX, 3, 1),
        (4, 3, 1),
        (65536, 0, 1),
        (65536, 11, 1),
        (65536, 3, 9),
        (1_048_577, 1, 1),
        (8191, 1, 1),
        (65536, 3, 0),
    ] {
        let (v, path) = created_with_secret();
        v.lock();
        set_params(&path, m, t, p);
        let v = FileVault::open(path);
        assert_eq!(
            v.unlock(sec(MASTER)),
            Err(SecretError::VaultCorrupt),
            "{m} {t} {p}"
        );
    }
}

/// `FileVault::open` refuses the tests' fast parameters; only `with_params(fast())` takes them.
#[test]
fn open_accepts_only_the_bounds() {
    let (v, path) = created_with_secret();
    v.lock();
    assert_eq!(
        FileVault::open(path.clone()).unlock(sec(MASTER)),
        Err(SecretError::VaultCorrupt)
    );
    assert!(
        FileVault::with_params(path, fast())
            .unlock(sec(MASTER))
            .is_ok()
    );
}

#[test]
fn master_shorter_than_8_is_refused() {
    assert!(vault_tmp().create(sec("short")).is_err());
    let (v, path) = vault_tmp_path();
    assert!(v.create(sec("1234567")).is_err());
    assert!(!path.exists() && !v.is_unlocked());
    // Characters, not bytes: eight two-byte letters are enough, seven are not.
    assert!(vault_tmp().create(sec("ğğğğğğğ")).is_err());
    assert!(vault_tmp().create(sec("ğğğğğğğğ")).is_ok());
}

#[test]
fn create_refuses_an_existing_file() {
    let (v, path) = created_with_secret();
    let before = std::fs::read(&path).unwrap();
    let again = FileVault::with_params(path.clone(), fast());
    assert!(again.exists());
    assert!(again.create(sec("another horse")).is_err());
    assert!(!again.is_unlocked());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(v.is_unlocked());
}

#[test]
fn every_write_uses_a_fresh_nonce() {
    let (v, path) = created_with_secret();
    let n1 = nonce_of(&path);
    let s1 = salt_of(&path);
    v.put(&key(), &sec("y")).unwrap();
    assert_ne!(nonce_of(&path), n1);
    assert_eq!(salt_of(&path), s1);
    let n2 = nonce_of(&path);
    v.delete(&key()).unwrap();
    assert!(nonce_of(&path) != n1 && nonce_of(&path) != n2);
    assert_eq!(salt_of(&path), s1);
}

#[test]
fn reset_deletes_file_and_locks() {
    let (v, path) = created_with_secret();
    v.reset().unwrap();
    assert!(!path.exists() && !v.is_unlocked() && !v.exists());
    // A new vault can be created in its place.
    v.create(sec("another horse")).unwrap();
    assert!(v.get(&key()).unwrap().is_none());
}

#[test]
fn reset_works_while_locked_and_on_a_damaged_file() {
    let (v, path) = created_with_secret();
    v.lock();
    truncate(&path, 10);
    v.reset().unwrap();
    assert!(!path.exists() && !v.exists());
}

#[test]
fn no_vault_holds_nothing_and_put_is_locked() {
    let (v, path) = vault_tmp_path();
    assert!(!v.exists() && !v.is_unlocked());
    assert!(v.get(&key()).unwrap().is_none());
    assert_eq!(v.delete(&key()), Ok(()));
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultLocked));
    assert!(!path.exists());
}

#[test]
fn lock_forgets_the_secrets() {
    let (v, _) = created_with_secret();
    v.lock();
    assert!(!v.is_unlocked());
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultLocked));
    assert_eq!(v.delete(&key()), Err(SecretError::VaultLocked));
}

#[test]
fn many_secrets_with_awkward_characters_round_trip() {
    let (v, _) = created_with_secret();
    let awkward = "q\"uo\\te\n\t\u{1}ğü€😀/";
    v.put(&other_key(), &sec(awkward)).unwrap();
    v.lock();
    v.unlock(sec(MASTER)).unwrap();
    assert_secret!(v.get(&other_key()).unwrap(), awkward);
    assert_secret!(v.get(&key()).unwrap(), "s3cret");
}

#[test]
fn debug_never_shows_master_or_values() {
    let v = created_with_secret().0;
    let shown = format!("{v:?}");
    assert!(!shown.contains("s3cret") && !shown.contains(MASTER));
    assert!(!shown.contains(&key().service()));
}

/// A share or device path is refused before any file system call (the `is_network_path`
/// rule: opening one on Windows can send the user's NTLM hash to a host).
#[test]
fn a_network_path_is_never_opened() {
    for p in [
        r"\\attacker\s\vault.bin",
        "//host/share/vault.bin",
        r"\\.\pipe\x",
    ] {
        let v = FileVault::with_params(PathBuf::from(p), fast());
        assert!(!v.exists(), "{p}");
        assert!(v.create(sec(MASTER)).is_err(), "{p}");
        assert!(v.unlock(sec(MASTER)).is_err(), "{p}");
        assert!(!v.is_unlocked(), "{p}");
        assert!(v.get(&key()).unwrap().is_none(), "{p}");
        assert!(v.put(&key(), &sec("x")).is_err(), "{p}");
        assert!(v.reset().is_err(), "{p}");
    }
    // On Unix a backslash path is a relative file name: nothing (not even a lock file) was
    // created for it in the working directory (fix round 1).
    #[cfg(unix)]
    for entry in std::fs::read_dir(".").unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with('\\'),
            "created {name:?}"
        );
    }
}

/// A symlink planted at `vault.bin` is never followed: the write refuses and the link's target
/// is untouched; the unlock refuses to read through it.
#[cfg(unix)]
#[test]
fn a_symlink_at_the_vault_path_is_refused() {
    let (v, path) = created_with_secret();
    let victim = tmp_dir().join("victim");
    std::fs::write(&victim, b"untouched").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    // Fix round 1: every write re-reads the file under the lock first, and a link is not a
    // vault file, so the write stops there without writing anything. ## m2b-t21-prompts (T6
    // review N3): a link is `Denied`, not `VaultCorrupt` (no reset is ever offered for it).
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::Denied));
    assert_eq!(std::fs::read(&victim).unwrap(), b"untouched");
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(v.get(&key()).is_err());

    // A link to a real vault is not read either.
    let (real, real_path) = created_with_secret();
    real.lock();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&real_path, &path).unwrap();
    v.lock();
    assert_eq!(v.unlock(sec(MASTER)), Err(SecretError::Denied));
}

// ## m2b-t21-prompts

/// T6 review N3: a symlink at `vault.bin` is never reset (the link stays, and so does what it
/// points to), and the vault says so before anyone offers a reset.
#[cfg(unix)]
#[test]
fn a_symlinked_vault_is_never_reset() {
    let (v, path) = created_with_secret();
    assert!(v.resettable(), "a regular vault file may be reset");
    let elsewhere = tmp_dir().join("dotfiles-vault.bin");
    std::fs::rename(&path, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert!(!v.resettable());
    assert_eq!(v.reset(), Err(SecretError::Denied));
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link stays"
    );
    assert!(elsewhere.exists(), "the file behind it stays");
    std::fs::remove_file(&path).unwrap();
    assert!(v.resettable(), "nothing at the path may be reset (a no-op)");
}

/// T6 review N2: a lock another instance holds for good makes a vault call give up after a
/// bounded wait (a write refuses with `VaultReadOnly`, which a Forever put falls back from)
/// instead of stalling the blocking pool forever.
#[cfg(unix)]
#[test]
fn a_held_lock_never_stalls_a_vault_call_forever() {
    let (made, path) = created_with_secret();
    drop(made);
    // Fix round 1 (m9): a short wait for the test, and a timed-out lock makes the next calls
    // fail fast instead of waiting again, slot after slot.
    let v = FileVault::with_params(path.clone(), fast())
        .with_lock_wait(std::time::Duration::from_millis(100));
    v.unlock(sec(MASTER)).unwrap();
    let mut lock_path = path.clone().into_os_string();
    lock_path.push(".lock");
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    holder.lock().unwrap();
    assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultReadOnly));
    // The next writes give up at once (one try each), so a delete of every slot stays short.
    for _ in 0..20 {
        assert_eq!(v.put(&key(), &sec("x")), Err(SecretError::VaultReadOnly));
    }
    holder.unlock().unwrap();
    v.put(&key(), &sec("x")).unwrap();
}

/// A stale temp file (or a symlink planted there) is replaced, never written through, and the
/// temp file never survives a write.
#[cfg(unix)]
#[test]
fn a_planted_temp_file_is_never_followed() {
    let (v, path) = created_with_secret();
    let victim = tmp_dir().join("victim");
    std::fs::write(&victim, b"untouched").unwrap();
    std::os::unix::fs::symlink(&victim, tmp_of(&path)).unwrap();
    v.put(&key(), &sec("y")).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"untouched");
    assert!(std::fs::symlink_metadata(tmp_of(&path)).is_err());
    assert_eq!(mode_of(&path), 0o600);
    v.lock();
    v.unlock(sec(MASTER)).unwrap();
    assert_secret!(v.get(&key()).unwrap(), "y");
}

/// A write that fails leaves the vault as it was, on disk and in memory.
#[cfg(unix)]
#[test]
fn a_failed_write_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (v, path) = created_with_secret();
    let dir = path.parent().unwrap().to_path_buf();
    let before = std::fs::read(&path).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    // Root ignores directory permissions: nothing to prove there.
    let writable = std::fs::write(dir.join("probe"), b"").is_ok();
    let put = v.put(&key(), &sec("y"));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if writable {
        return;
    }
    assert_eq!(put, Err(SecretError::VaultReadOnly));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_secret!(v.get(&key()).unwrap(), "s3cret");
}

// ## m2b-t06-vault fix round 1 (I1, I2, M4): several instances on one vault file.

/// Another instance on the same file (a second Hacivat process until M8's single-instance
/// lock), unlocked.
fn second_instance(path: &Path) -> FileVault {
    let v = FileVault::with_params(path.to_path_buf(), fast());
    v.unlock(sec(MASTER)).expect("the second instance unlocks");
    v
}

/// A key of its own for every `n`.
fn nth_key(n: u8) -> SecretKey {
    SecretKey {
        slot: SecretSlot::SshPassphrase { hop: n % 9 },
        profile: format!("0b0f5a3c-2f7e-4a54-9d3b-6c1e8f2a7d{n:02x}")
            .parse()
            .expect("a valid UUID"),
    }
}

/// Every temp file left next to the vault.
fn temps_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

#[test]
fn two_instances_interleaving_saves_keep_both_secrets() {
    let (a, path) = created_with_secret();
    let b = second_instance(&path);
    a.put(&other_key(), &sec("from-a")).unwrap();
    b.put(&nth_key(1), &sec("from-b")).unwrap();
    a.put(&nth_key(2), &sec("from-a-again")).unwrap();
    // Each instance sees the other's writes, and a fresh unlock sees all of them.
    assert_secret!(a.get(&nth_key(1)).unwrap(), "from-b");
    assert_secret!(b.get(&nth_key(2)).unwrap(), "from-a-again");
    let fresh = second_instance(&path);
    assert_secret!(fresh.get(&key()).unwrap(), "s3cret");
    assert_secret!(fresh.get(&other_key()).unwrap(), "from-a");
    assert_secret!(fresh.get(&nth_key(1)).unwrap(), "from-b");
    assert_secret!(fresh.get(&nth_key(2)).unwrap(), "from-a-again");
    // A delete in one instance is not undone by the other's next write.
    b.delete(&other_key()).unwrap();
    a.put(&nth_key(3), &sec("z")).unwrap();
    assert!(second_instance(&path).get(&other_key()).unwrap().is_none());
}

#[test]
fn a_reset_elsewhere_is_never_undone() {
    let (a, path) = created_with_secret();
    let b = second_instance(&path);
    b.reset().unwrap();
    b.create(sec("brand new horse")).unwrap();
    b.put(&nth_key(1), &sec("new")).unwrap();
    // A still holds the old key: its write must not bring the old vault back.
    assert_eq!(
        a.put(&other_key(), &sec("x")),
        Err(SecretError::VaultLocked)
    );
    assert!(!a.is_unlocked());
    assert!(matches!(a.get(&key()), Err(SecretError::VaultLocked)));
    assert_eq!(a.unlock(sec(MASTER)), Err(SecretError::WrongMasterPassword));
    let fresh = FileVault::with_params(path.clone(), fast());
    fresh.unlock(sec("brand new horse")).unwrap();
    assert_secret!(fresh.get(&nth_key(1)).unwrap(), "new");
    assert!(fresh.get(&key()).unwrap().is_none());

    // A reset with no new vault: the old instance holds nothing and writes nothing.
    let (a, path) = created_with_secret();
    second_instance(&path).reset().unwrap();
    assert!(a.get(&key()).unwrap().is_none());
    assert_eq!(a.put(&key(), &sec("x")), Err(SecretError::VaultLocked));
    assert_eq!(a.delete(&key()), Ok(()));
    assert!(!path.exists());
}

/// Two instances writing at once: no write fails on a temp-file collision, nothing is lost
/// and no temp file survives.
#[test]
fn concurrent_writers_never_collide_or_lose_writes() {
    let (a, path) = created_with_secret();
    let b = second_instance(&path);
    let per_writer = 25u8;
    std::thread::scope(|s| {
        for (offset, v) in [(0u8, &a), (100u8, &b)] {
            s.spawn(move || {
                for n in 0..per_writer {
                    assert_eq!(v.put(&nth_key(offset + n), &sec("w")), Ok(()));
                }
            });
        }
    });
    let fresh = second_instance(&path);
    for n in (0..per_writer).chain(100..100 + per_writer) {
        assert_secret!(fresh.get(&nth_key(n)).unwrap(), "w");
    }
    assert_secret!(fresh.get(&key()).unwrap(), "s3cret");
    assert!(temps_in(path.parent().unwrap()).is_empty());
}

/// A healthy vault replaced atomically, by a writer that does not take the lock, is never
/// reported `VaultCorrupt` (I1): every unlock succeeds.
#[test]
fn a_racing_replace_is_never_corrupt() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (v, path) = created_with_secret();
    v.lock();
    let bytes = std::fs::read(&path).unwrap();
    let done = AtomicBool::new(false);
    let scratch = path.with_file_name("racer");
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::SeqCst) {
                std::fs::write(&scratch, &bytes).unwrap();
                std::fs::rename(&scratch, &path).unwrap();
            }
        });
        let outcomes: Vec<_> = (0..300)
            .map(|_| {
                let unlocked = v.unlock(sec(MASTER));
                v.lock();
                unlocked
            })
            .collect();
        done.store(true, Ordering::SeqCst);
        assert!(!outcomes.contains(&Err(SecretError::VaultCorrupt)));
        assert!(outcomes.iter().all(Result::is_ok), "{outcomes:?}");
    });
}

/// A vault that cannot be looked at (an unreadable directory) is an error, never "no vault":
/// a locked vault's secret is never reported deleted or absent (M4).
#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_an_error_not_no_vault() {
    use std::os::unix::fs::PermissionsExt;
    let (v, path) = created_with_secret();
    v.lock();
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root ignores directory permissions: nothing to prove there.
    let blind = std::fs::symlink_metadata(&path).is_err();
    let get = v.get(&key()).map(|_| ());
    let delete = v.delete(&key());
    let exists = v.exists();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if !blind {
        return;
    }
    assert_eq!(get, Err(SecretError::Denied));
    assert_eq!(delete, Err(SecretError::Denied));
    assert!(exists, "an unknown state is never \"no vault\"");
    v.unlock(sec(MASTER)).unwrap();
    assert_secret!(v.get(&key()).unwrap(), "s3cret");
}

/// Every write uses a temp file of its own, and none survives; the lock file is owner-only.
#[test]
fn temp_files_are_per_write_and_never_survive() {
    let (v, path) = created_with_secret();
    for n in 0..5 {
        v.put(&nth_key(n), &sec("t")).unwrap();
    }
    assert!(temps_in(path.parent().unwrap()).is_empty());
    #[cfg(unix)]
    assert_eq!(mode_of(&path.with_file_name("vault.bin.lock")), 0o600);
}
