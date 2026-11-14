//! Complexity counters (L2: PR CI asserts counters, never wall time) and the release perf
//! budget (Task 4; `alg/10` LEX-02 "Complexity", digest 01 R5).

use std::time::{Duration, Instant};

use sql_dialect::{DialectCtx, LexDialect, LexModes};
use sql_lexer::{LexState, Lexed, TokenFlags, TokenKind, lex, lex_counting_steps};
use sql_testkit::lex::{dialect_ctx, profile};
use sql_text::{TextRange, TextSize};

fn pg() -> DialectCtx {
    dialect_ctx("postgres", None)
}

fn pg_profile() -> &'static LexDialect {
    profile(&pg())
}

fn modes() -> LexModes {
    pg().modes
}

fn lex_pg(text: &str) -> Lexed {
    lex(text, pg_profile(), modes(), &LexState::Initial)
}

/// The bytes the cursor examined lexing `text` in `ctx`.
fn steps_in(ctx: &DialectCtx, text: &str) -> u64 {
    lex_counting_steps(text, profile(ctx), ctx.modes, &LexState::Initial).1
}

fn steps(text: &str) -> u64 {
    steps_in(&pg(), text)
}

#[test]
fn million_deep_comment_does_not_recurse() {
    let s = "/*".repeat(1_000_000) + &"*/".repeat(1_000_000);
    let l = lex(&s, pg_profile(), modes(), &LexState::Initial);
    assert_eq!(l.len(), 1);
    assert_eq!(l.kind(0), TokenKind::BlockComment);
    assert!(!l.flags(0).contains(TokenFlags::UNTERMINATED));
    // One more opener: unterminated a million levels deep, still one linear pass.
    let s = "/*".repeat(1_000_001) + &"*/".repeat(1_000_000);
    let l = lex(&s, pg_profile(), modes(), &LexState::Initial);
    assert_eq!(*l.end_state(), LexState::InBlockComment { depth: 1 });
}

#[test]
fn token_at_and_tokens_in_are_logarithmic() {
    let l = lex_pg(&"SELECT 1;\n".repeat(50_000));
    assert_eq!(l.len(), 250_000);
    let (found, probes) = l.token_at_probed(TextSize::from(250_000));
    assert!(probes <= 20, "{probes} probes");
    assert_eq!(found, Some(125_000));
    assert_eq!(found, l.token_at(TextSize::from(250_000)));
    let r = TextRange::new(TextSize::from(400_000), TextSize::from(400_020));
    let (tokens, probes) = l.tokens_in_probed(r);
    assert!(probes <= 40, "{probes} probes");
    assert_eq!(tokens, l.tokens_in(r));
    assert_eq!(tokens, 200_000..200_010);
}

#[test]
fn lexing_is_linear() {
    let small = steps(&"SELECT 1;".repeat(1_000));
    let big = steps(&"SELECT 1;".repeat(100_000));
    assert!(big <= small * 110, "{small} → {big}");
}

/// Inputs that would turn a careless lexer quadratic; each must stay within a constant number
/// of steps per byte at 100× the size.
#[test]
fn adversarial_inputs_stay_linear() {
    let mysql = dialect_ctx("mysql", None);
    let sqlite = dialect_ctx("sqlite", None);
    let cases: [(&DialectCtx, &str); 16] = [
        (&pg(), "+-"),     // operator run split one sign at a time
        (&pg(), "*+-+-"),  // a run with a non-sign, split repeatedly
        (&pg(), "?-+"),    // a sign keeper
        (&pg(), "$a "),    // failed dollar tags, re-read as words
        (&pg(), "$a$ "),   // dollar quotes closing one another
        (&pg(), "/*"),     // unterminated nesting
        (&pg(), "'"),      // doubled quotes
        (&pg(), "E'\\"),   // escapes
        (&pg(), "1e"),     // junk after numbers
        (&pg(), ":'a"),    // failed psql variables
        (&pg(), "0x_"),    // failed radix integers
        (&mysql, "1e1"),   // a digit-leading word re-read
        (&mysql, "/*!0 "), // unterminated executable comment openers
        (&sqlite, "$a("),  // Tcl suffixes that never close
        (&sqlite, "$a::"), // Tcl segments
        (&mysql, "@@a."),  // variables
    ];
    for (ctx, unit) in cases {
        let small = steps_in(ctx, &unit.repeat(1_000));
        let big = steps_in(ctx, &unit.repeat(100_000));
        assert!(big <= small * 110, "{unit:?}: {small} → {big}");
        let per_byte = big as f64 / (unit.len() * 100_000) as f64;
        assert!(per_byte <= 8.0, "{unit:?}: {per_byte:.1} steps per byte");
    }
}

#[test]
fn ten_megabyte_string_is_one_token() {
    let s = format!("SELECT '{}'", "çğ;x".repeat(1_750_000));
    assert!(s.len() > 10_000_000);
    let (l, steps) = lex_counting_steps(&s, pg_profile(), modes(), &LexState::Initial);
    assert_eq!(l.len(), 3);
    assert_eq!(l.kind(2), TokenKind::String);
    assert!(steps <= 2 * s.len() as u64, "{steps} steps");
}

/// A 1 MB mix of the shapes `cargo xtask gen-sql` writes (harman queries, CTEs, block
/// comments with Turkish text, long `IN` lists, inserts), deterministic.
fn gen_sql(bytes: usize) -> std::string::String {
    const CITIES: [&str; 4] = ["İstanbul", "Iğdır", "Şanlıurfa", "Çanakkale"];
    let mut seed: u64 = 0x4841_4349_5641_5442;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut sql = std::string::String::with_capacity(bytes + 1024);
    while sql.len() < bytes {
        let city = CITIES[usize::try_from(next(4)).unwrap()];
        match next(5) {
            0 => sql.push_str(&format!(
                "-- Top customers, last {} days\nSELECT c.full_name, c.city, count(o.id) AS orders,\n       sum(oi.line_total) AS revenue\nFROM customers c\nJOIN orders o ON o.customer_id = c.id\nWHERE o.ordered_at > now() - interval '{} days'\nGROUP BY c.full_name, c.city\nORDER BY revenue DESC\nLIMIT {};\n\n",
                next(90), next(90), 10 * (1 + next(20))
            )),
            1 => sql.push_str(&format!(
                "WITH recent AS (\n    SELECT o.id, o.total FROM orders o WHERE o.ordered_at >= current_date - {}\n)\nSELECT city, sum(total) FROM recent r JOIN customers c ON c.id = r.id\nWHERE city IN ('{city}', 'Muğla')\nGROUP BY city;\n",
                next(60)
            )),
            2 => sql.push_str(&format!(
                "/*\n * Nightly reconciliation for batch {}.\n * Müşteri kayıtları {city} şubesinden gelir.\n */\n",
                next(100_000)
            )),
            3 => {
                sql.push_str("UPDATE orders SET status = 'cancelled' WHERE id IN (\n");
                for row in 0..8 {
                    let ids: Vec<std::string::String> =
                        (0..10).map(|_| (100_000 + next(900_000)).to_string()).collect();
                    sql.push_str("    ");
                    sql.push_str(&ids.join(", "));
                    sql.push_str(if row < 7 { ",\n" } else { "\n);\n" });
                }
            }
            _ => sql.push_str(&format!(
                "INSERT INTO customers (full_name, city) VALUES ('Ayşe Yılmaz', '{city}');\n"
            )),
        }
    }
    sql
}

#[test]
#[ignore = "perf"]
fn one_megabyte_lexes_within_10_ms_release() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    let sql = gen_sql(1 << 20);
    for ctx in [
        pg(),
        dialect_ctx("mysql", None),
        dialect_ctx("sqlite", None),
    ] {
        let p = profile(&ctx);
        let mut times: Vec<Duration> = (0..10)
            .map(|_| {
                let start = Instant::now();
                let l = lex(&sql, p, ctx.modes, &LexState::Initial);
                let took = start.elapsed();
                assert!(l.len() > 100_000);
                took
            })
            .collect();
        times.sort();
        let median = times[5];
        let mb_s = sql.len() as f64 / median.as_secs_f64() / 1e6;
        println!("{}: 1 MiB median {median:?} ({mb_s:.0} MB/s)", p.name());
        assert!(
            median <= Duration::from_millis(10),
            "{}: {median:?}",
            p.name()
        );
    }
}

#[test]
fn the_perf_fixture_is_a_megabyte_of_sql() {
    let sql = gen_sql(1 << 20);
    assert!((1 << 20..(1 << 20) + 2048).contains(&sql.len()));
    assert_eq!(sql, gen_sql(1 << 20));
    let l = lex_pg(&sql);
    assert!(l.errors().is_empty());
    assert_eq!(*l.end_state(), LexState::Initial);
}
