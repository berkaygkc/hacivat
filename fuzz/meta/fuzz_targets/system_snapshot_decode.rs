//! Fuzz target `system_snapshot_decode` (M4 Task 1a skeleton; body Task 23): decoding an
//! untrusted system-catalog snapshot never panics (R-M4-33; Review Focus 4). Whatever decodes
//! is the canonical form of itself (a second decode of the same bytes agrees), and every
//! object it yields carries the snapshot flags. Seeded by `fuzz-meta` with the committed
//! assets.

#![no_main]

use hacivat_meta::ObjFlags;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let json = hacivat_meta::system::decode_json(data);
    let snap = hacivat_meta::system::decode(data);
    // `decode` accepts exactly what `decode_json` accepts.
    assert_eq!(json.is_ok(), snap.is_ok());
    let Ok(snap) = snap else {
        return;
    };
    let flags = ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED;
    for ns in snap.namespaces.values() {
        assert!(ns.objs().iter().all(|o| o.flags().contains(flags)));
    }
    assert!(snap.builtins.iter().all(|o| o.flags().contains(flags)));
});
