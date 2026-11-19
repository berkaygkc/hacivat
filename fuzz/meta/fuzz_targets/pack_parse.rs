//! Fuzz target `pack_parse` (M4 Task 4): parsing a query pack (HV-INTRO-04), linting it and
//! rendering every section for a spread of contexts never panics, and a list bind never
//! yields more queries than chunks of `MAX_IN_LIST` items. The first byte picks the dialect and the list
//! length (up to 2,520 items, so chunk boundaries are reached); the rest is the pack text.
//! Built without `hacivat-introspect`'s `session` feature: rendering is session-free, only
//! `RenderedQuery::as_meta_query` is not (ADR 0008 `m4-t04`).

#![no_main]

use std::sync::Arc;

use hacivat_introspect::pack::{
    BindValue, DialectId, Flavor, LogicalType, MAX_IN_LIST, Mode, Pack, PackCtx, PackFlags,
    Params, lint,
};
use hacivat_meta::Level;
use libfuzzer_sys::fuzz_target;

fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// Bind names a fuzzed pack is likely to use; every one is bound both ways.
const NAMES: [&str; 8] = ["a", "b", "n", "s", "t", "x", "ids", "names"];

fuzz_target!(|data: &[u8]| {
    let Some((&sel, rest)) = data.split_first() else {
        return;
    };
    let Ok(src) = std::str::from_utf8(rest) else {
        return;
    };
    let (dialect, flavor) = match sel & 3 {
        0 => (DialectId::Postgres, Flavor::Postgres),
        1 => (DialectId::MySql, Flavor::MySql),
        2 => (DialectId::MariaDb, Flavor::MariaDb),
        _ => (DialectId::Sqlite, Flavor::Sqlite),
    };
    let Ok(pack) = Pack::parse(dialect, src) else {
        return;
    };
    let _ = lint(&pack, dialect);
    let items: Vec<BindValue> = (0..usize::from(sel >> 2) * 40)
        .map(|i| text(&i.to_string()))
        .collect();
    let mut params = Params::default();
    for (k, n) in NAMES.iter().enumerate() {
        let v = match k % 3 {
            0 => text("a`b"),
            1 => BindValue::Null,
            _ => text("t\0"),
        };
        params.scalars.push((Arc::from(*n), v));
        params.lists.push((Arc::from(*n), items.clone()));
    }
    for s in pack.sections() {
        let _ = pack.section(s);
        for version in [0, 150_000, u32::MAX] {
            for mode in [Mode::Full, Mode::Incremental, Mode::Brief, Mode::System] {
                for flags in [PackFlags::empty(), PackFlags::JOINED | PackFlags::MARIADB] {
                    let ctx = PackCtx {
                        dialect,
                        version,
                        flavor,
                        mode,
                        level: Level::Sources,
                        flags,
                        params: params.clone(),
                    };
                    if let Ok(qs) = pack.render(s, &ctx) {
                        // One query, or one per chunk of at most MAX_IN_LIST items.
                        assert!(qs.len() <= items.len().div_ceil(MAX_IN_LIST).max(1));
                    }
                }
            }
        }
    }
});
