//! The B3 fixture of `cargo xtask gen-sql` (`xtask/src/gen_sql.rs`), byte for byte: the same
//! SplitMix64 seed and shapes. The SQL crates cannot depend on `xtask`, so the incremental
//! perf tests (R-M3-9, Task 5) carry this copy; `gen_matches_the_xtask_shape` pins its size.

#![allow(dead_code)]

use std::fmt::Write as _;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

const CITIES: &[&str] = &[
    "İstanbul",
    "Iğdır",
    "Şanlıurfa",
    "Çanakkale",
    "Muğla",
    "Ağrı",
    "Kırşehir",
    "Gümüşhane",
];
const NAMES: &[&str] = &[
    "Ayşe Yılmaz",
    "Çağrı Öztürk",
    "İlker Şahin",
    "Gülşen Doğan",
    "Ümit Kılıç",
];
const SEGMENTS: &[&str] = &["retail", "wholesale", "enterprise", "partner"];
const STATUSES: &[&str] = &["paid", "pending", "shipped", "cancelled", "refunded"];

fn harman_top_customers(r: &mut Rng, out: &mut Vec<String>) {
    let days = 7 + r.below(83);
    let limit = 10 * (1 + r.below(20));
    out.extend([
        format!("-- Top customers by revenue, last {days} days"),
        "SELECT c.full_name,".into(),
        "       c.city, c.segment,".into(),
        "       count(o.id)          AS orders,".into(),
        "       sum(oi.line_total)   AS revenue".into(),
        "FROM customers c".into(),
        "JOIN orders o ON o.customer_id = c.id".into(),
        "JOIN order_items oi ON oi.order_id = o.id".into(),
        format!("WHERE o.ordered_at > now() - interval '{days} days'"),
        "GROUP BY c.full_name, c.city, c.segment".into(),
        "ORDER BY revenue DESC".into(),
        format!("LIMIT {limit};"),
        String::new(),
    ]);
}

fn harman_status_filter(r: &mut Rng, out: &mut Vec<String>) {
    let status = r.pick(STATUSES);
    let id = 1000 + r.below(900_000);
    out.push(format!(
        "SELECT * FROM orders WHERE status = '{status}' AND customer_id = {id};"
    ));
}

fn cte(r: &mut Rng, out: &mut Vec<String>) {
    let days = 1 + r.below(60);
    let cities: Vec<String> = (0..4).map(|_| format!("'{}'", r.pick(CITIES))).collect();
    out.extend([
        "WITH recent_orders AS (".into(),
        "    SELECT o.id, o.customer_id, o.total".into(),
        "    FROM orders o".into(),
        format!("    WHERE o.ordered_at >= current_date - {days}"),
        "),".into(),
        "city_totals AS (".into(),
        "    SELECT c.city, sum(r.total) AS total".into(),
        "    FROM recent_orders r".into(),
        "    JOIN customers c ON c.id = r.customer_id".into(),
        "    GROUP BY c.city".into(),
        ")".into(),
        "SELECT city, total".into(),
        "FROM city_totals".into(),
        format!("WHERE city IN ({})", cities.join(", ")),
        "ORDER BY total DESC;".into(),
        String::new(),
    ]);
}

fn block_comment(r: &mut Rng, out: &mut Vec<String>) {
    let batch = r.below(100_000);
    out.extend([
        "/*".into(),
        format!(" * Nightly reconciliation for batch {batch}."),
        format!(
            " * Müşteri kayıtları {} ve {} şubelerinden gelir.",
            r.pick(CITIES),
            r.pick(CITIES)
        ),
        " */".into(),
    ]);
}

fn long_in_list(r: &mut Rng, out: &mut Vec<String>) {
    out.push("UPDATE orders".into());
    out.push("SET status = 'cancelled', cancelled_at = now()".into());
    out.push("WHERE id IN (".into());
    let rows = 4 + r.below(24);
    for row in 0..rows {
        let mut line = String::from("    ");
        for i in 0..10 {
            if i > 0 {
                line.push_str(", ");
            }
            let _ = write!(line, "{}", 100_000 + r.below(900_000));
        }
        if row + 1 < rows {
            line.push(',');
        }
        out.push(line);
    }
    out.push(");".into());
}

fn insert_customer(r: &mut Rng, out: &mut Vec<String>) {
    out.push("INSERT INTO customers (full_name, city, segment)".into());
    out.push(format!(
        "VALUES ('{}', '{}', '{}');",
        r.pick(NAMES),
        r.pick(CITIES),
        r.pick(SEGMENTS)
    ));
}

/// Exactly `lines` lines of the B3 fixture, each ending in `\n`.
pub fn gen_sql(lines: usize) -> String {
    let mut r = Rng(0x4841_4349_5641_5442); // "HACIVATB"
    let mut out: Vec<String> = Vec::with_capacity(lines + 64);
    while out.len() < lines {
        match r.below(16) {
            0..=3 => harman_top_customers(&mut r, &mut out),
            4..=6 => harman_status_filter(&mut r, &mut out),
            7..=9 => cte(&mut r, &mut out),
            10 | 11 => block_comment(&mut r, &mut out),
            12 | 13 => long_in_list(&mut r, &mut out),
            _ => insert_customer(&mut r, &mut out),
        }
    }
    out.truncate(lines);
    let mut sql = String::with_capacity(lines * 48);
    for line in &out {
        sql.push_str(line);
        sql.push('\n');
    }
    sql
}

/// One minified statement repeated to exactly `chars` characters, with no newline (`gen-sql
/// --long-line`).
pub fn minified_line(chars: usize) -> String {
    const STMT: &str = "SELECT c.full_name,c.city,count(o.id) AS orders FROM customers c JOIN orders o ON o.customer_id=c.id WHERE c.city IN ('İstanbul','Iğdır') AND o.id IN (100231,100232,100233) GROUP BY c.full_name,c.city;";
    STMT.chars().cycle().take(chars).collect()
}

/// A deterministic PRNG for the perf tests' edit positions (SplitMix64).
pub struct Positions(u64);

impl Positions {
    pub fn new(seed: u64) -> Positions {
        Positions(seed)
    }

    pub fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

/// The 95th percentile of `v` (in place).
pub fn p95(v: &mut [std::time::Duration]) -> std::time::Duration {
    v.sort_unstable();
    v[(v.len() * 95 / 100).min(v.len() - 1)]
}

/// Whether the perf tests run (`HACIVAT_PERF=1`, P9).
pub fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}
