//! Fuzz target `classify_no_panic` (SQL-T13; Task 20): `sql_safety::classify` never panics,
//! on every dialect, with the lexing modes unknown and known, for one statement or a script,
//! and its report is well formed: of exactly these bytes, in this dialect, pre-decided with no
//! item or with every item inside the text, and nothing the read-only profile allows is more
//! than a read or a harmless session change. The seeded PR form is
//! `sql-safety/tests/mutation_props.rs` (`classify_never_panics_and_reports_are_well_formed`);
//! the seed corpus is `sql-testkit/corpus/fuzz/classifier_seeds.toml`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, PgWireFlavor};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, Expect, Profile, classify, decide, spawn_classifier_thread,
};

fn check(text: &str) {
    for d in [
        DialectId::Postgres,
        DialectId::PgWire(PgWireFlavor::Redshift),
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        for expect in [Expect::SingleStatement, Expect::Script] {
            let unknown = ClassifyCtx::new(DialectCtx::new(d, None), ClientFlavor::None, expect);
            let known = unknown.clone().with_known_modes(LexModes::default_for(d));
            for ctx in [unknown, known] {
                let r = classify(text, &ctx);
                assert_eq!(r.dialect, Some(d));
                assert!(r.matches_text(text), "{d:?}: a report of other bytes");
                if r.pre_decision.is_some() {
                    assert!(r.items.is_empty() && r.request_class == CoarseClass::Unknown);
                } else {
                    assert!(!r.items.is_empty(), "{d:?}: no pre-decision and no item");
                }
                for i in &r.items {
                    assert!(
                        usize::from(i.range.end()) <= text.len(),
                        "{d:?}: item outside"
                    );
                }
                if decide(&r, Profile::ReadOnlyPlusExplain, Actor::Human).allows() {
                    assert!(
                        r.request_class <= CoarseClass::Session,
                        "{d:?}: allowed {r:?}"
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let text = text.to_owned();
    // The classifier's own stack (16 MiB), as the pool runs it.
    let h = spawn_classifier_thread("fuzz-classify".to_string(), move || check(&text))
        .expect("a classifier thread");
    if let Err(p) = h.join() {
        std::panic::resume_unwind(p);
    }
});
