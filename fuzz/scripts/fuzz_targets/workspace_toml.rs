//! Fuzz target `workspace_toml` (M4b; body: Task 20): reading an untrusted
//! `.hacivat/workspace.toml` (HV-SCRIPT-01 step 3) never panics, whatever the bytes.
//!
//! Oracles (Task 20):
//! - a bound name (`database` or an alias) passes the character rules: non-empty, at most
//!   `names::MAX_NAME_BYTES`, no control or bidi control character (`LogicalDb::parse` accepts it
//!   again unchanged);
//! - at most `MAX_ALIASES` aliases, none equal to the database or to another alias;
//! - the size limit holds: past `MANIFEST_MAX_BYTES` nothing binds;
//! - no key outside the schema binds anything: bytes that never spell `database` (and use no
//!   `\u`/`\U` escape that could spell it inside a quoted key) bind no database.

#![no_main]

use hacivat_scripts::manifest::{self, MANIFEST_MAX_BYTES, MAX_ALIASES};
use hacivat_scripts::names::{LogicalDb, MAX_NAME_BYTES};
use libfuzzer_sys::fuzz_target;

/// The character rules a bound name must pass, checked apart from the parser.
fn passes_rules(db: &LogicalDb) {
    let s = db.as_str();
    assert!(!s.is_empty(), "an empty name bound");
    assert!(s.len() <= MAX_NAME_BYTES, "a name over the limit bound");
    assert!(
        !s.chars().any(|c| c.is_control()
            || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')),
        "a control or bidi control character bound: {s:?}"
    );
    let again = LogicalDb::parse(s).expect("a bound name parses again");
    assert_eq!(again.as_str(), s, "a bound name is stored in its normal form");
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fuzz_target!(|data: &[u8]| {
    let read = manifest::read_bytes(data, "library");
    if data.len() > MANIFEST_MAX_BYTES {
        assert!(read.db.is_none() && read.aliases.is_empty(), "an oversized manifest bound");
    }
    if let Some(db) = &read.db {
        passes_rules(db);
    }
    assert!(read.aliases.len() <= MAX_ALIASES, "{} aliases", read.aliases.len());
    for (i, a) in read.aliases.iter().enumerate() {
        passes_rules(a);
        if let Some(db) = &read.db {
            assert!(!a.same_db(db), "an alias equal to the database: {a:?}");
        }
        assert!(
            read.aliases[..i].iter().all(|b| !b.same_db(a)),
            "a duplicate alias: {a:?}"
        );
    }
    if !contains(data, b"database") && !contains(data, b"\\u") && !contains(data, b"\\U") {
        assert!(read.db.is_none(), "a database bound by a key outside the schema");
    }
});
