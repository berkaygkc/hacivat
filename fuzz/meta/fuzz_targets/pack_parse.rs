//! Fuzz target `pack_parse` (M4 Task 4): parsing a query pack (HV-INTRO-04), linting it and
//! rendering every section for a spread of contexts never panics, and every render passes
//! three oracles (fix round 1, m4):
//! - re-lexed with the server's own literal and comment rules, the rendered text holds no `;`,
//!   exactly as many placeholders as binds, and on PostgreSQL `$1..$n` in order;
//! - a `show-create` render holds no placeholder and its backtick-quoted names double every
//!   inner backtick;
//! - parse → render is idempotent (a second parse and render give the same packs and text),
//!   and a list bind never yields more queries than chunks of `MAX_IN_LIST` items.
//!
//! The first byte picks the dialect and the list length (up to 2,520 items, so chunk
//! boundaries are reached); the rest is the pack text, split into two files at the first
//! U+0001 when there is one (the multi-file merge). Built without `hacivat-introspect`'s
//! `session` feature: runtime text parses into a `DraftPack` (ADR 0008 `m4-t04`).

#![no_main]

use std::sync::Arc;

use hacivat_introspect::pack::{
    BindValue, DialectId, DraftPack, DraftQuery, Flavor, LogicalType, MAX_IN_LIST, Mode,
    PackCtx, PackFlags, Params, lint,
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

/// The rendered statement as the server lexes it: placeholders outside literals, quoted
/// identifiers and comments, in order (PG `$n` numbers; `?` counts as 0), and whether a `;`
/// stands outside them. `None` if a literal or comment does not close (never after a parse).
fn relex(sql: &str, pg: bool) -> Option<(Vec<usize>, bool)> {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut places = Vec::new();
    let mut semi = false;
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"' | b'`') if !(pg && q == b'`') => {
                let mut j = i + 1;
                loop {
                    if j >= b.len() {
                        return None;
                    }
                    if b[j] == q {
                        if b.get(j + 1) == Some(&q) {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    j += 1;
                }
                i = j + 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = sql[i + 2..].find("*/")?;
                i += end + 4;
            }
            b';' => {
                semi = true;
                i += 1;
            }
            b'?' if !pg => {
                places.push(0);
                i += 1;
            }
            b'$' if pg => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                places.push(sql[i + 1..j].parse().unwrap_or(0));
                i = j;
            }
            _ => i += 1,
        }
    }
    Some((places, semi))
}

fn check(q: &DraftQuery, pg: bool, show_create: bool) {
    let (places, semi) = relex(q.sql(), pg).expect("a render re-lexes");
    assert!(!semi, "a stray statement break: {}", q.sql());
    assert_eq!(places.len(), q.binds().len(), "{}", q.sql());
    if pg {
        assert!(places.iter().enumerate().all(|(k, n)| *n == k + 1), "{}", q.sql());
    }
    if show_create {
        assert!(places.is_empty());
        // Every backtick-quoted name closes where the renderer closed it.
        let mut in_q = false;
        let b = q.sql().as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                if in_q && b.get(i + 1) == Some(&b'`') {
                    i += 2;
                    continue;
                }
                in_q = !in_q;
            }
            i += 1;
        }
        assert!(!in_q, "{}", q.sql());
    }
}

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
    let parse = || match src.split_once('\u{1}') {
        Some((a, b)) => DraftPack::parse_all(dialect, &[("20-b", b), ("10-a", a)]),
        None => DraftPack::parse(dialect, src),
    };
    let Ok(pack) = parse() else {
        return;
    };
    assert_eq!(parse().as_ref(), Ok(&pack), "parsing is deterministic");
    let again = parse().expect("parses again");
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
    let pg = dialect == DialectId::Postgres;
    for s in pack.sections() {
        let show_create = pack.section(s).is_some_and(|x| x.show_create());
        for version in [0, 150_000, u32::MAX] {
            for mode in [Mode::Full, Mode::Incremental, Mode::Brief, Mode::System] {
                // `render` refuses a MARIADB flag that disagrees with the flavor.
                let maria = if flavor == Flavor::MariaDb {
                    PackFlags::MARIADB
                } else {
                    PackFlags::empty()
                };
                for flags in [maria, PackFlags::JOINED | maria] {
                    let ctx = PackCtx {
                        dialect,
                        version,
                        flavor,
                        mode,
                        level: Level::Sources,
                        flags,
                        params: params.clone(),
                    };
                    let first = pack.render(s, &ctx);
                    assert_eq!(first, again.render(s, &ctx), "render is idempotent");
                    if let Ok(qs) = first {
                        // One query, or one per chunk of at most MAX_IN_LIST items.
                        assert!(qs.len() <= items.len().div_ceil(MAX_IN_LIST).max(1));
                        for q in &qs {
                            check(q, pg, show_create);
                        }
                    }
                }
            }
        }
    }
});
