//! `cargo xtask gen-sql --lines <n> --out <path> [--long-line <chars>]`: writes the B3
//! fixture, a deterministic SQL file mixing the harman query shapes, CTEs, comments,
//! Turkish string literals and long `IN (…)` lists.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

pub fn run() -> ExitCode {
    match run_inner(std::env::args().skip(2).collect()) {
        Ok((path, bytes, lines)) => {
            println!("wrote {} ({bytes} bytes, {lines} lines)", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-sql: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: Vec<String>) -> Result<(PathBuf, usize, usize), String> {
    let (mut lines, mut out, mut long_line) = (None, None, None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--lines" => lines = Some(value()?.parse::<usize>().map_err(|e| e.to_string())?),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--long-line" => {
                long_line = Some(value()?.parse::<usize>().map_err(|e| e.to_string())?)
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let lines = lines.ok_or("missing --lines <n>")?;
    let out = out.ok_or("missing --out <path>")?;
    let sql = r#gen(lines, long_line);
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&out, &sql).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok((out, sql.len(), sql.lines().count()))
}

/// SplitMix64: a tiny deterministic PRNG, so the fixture never depends on a crate version.
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
    out.extend(
        [
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
        ]
        .into_iter(),
    );
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
    out.extend(
        [
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
        ]
        .into_iter(),
    );
}

fn block_comment(r: &mut Rng, out: &mut Vec<String>) {
    let batch = r.below(100_000);
    out.extend(
        [
            "/*".into(),
            format!(" * Nightly reconciliation for batch {batch}."),
            format!(
                " * Müşteri kayıtları {} ve {} şubelerinden gelir.",
                r.pick(CITIES),
                r.pick(CITIES)
            ),
            " */".into(),
        ]
        .into_iter(),
    );
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

/// One minified statement repeated to exactly `chars` characters, with no newline.
fn minified_line(chars: usize) -> String {
    const STMT: &str = "SELECT c.full_name,c.city,count(o.id) AS orders FROM customers c JOIN orders o ON o.customer_id=c.id WHERE c.city IN ('İstanbul','Iğdır') AND o.id IN (100231,100232,100233) GROUP BY c.full_name,c.city;";
    STMT.chars().cycle().take(chars).collect()
}

/// Exactly `lines` lines, each ending in `\n`, plus one minified line of `long_line` chars.
pub fn r#gen(lines: usize, long_line: Option<usize>) -> String {
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
    if let Some(chars) = long_line {
        sql.push_str(&minified_line(chars));
        sql.push('\n');
    }
    sql
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_sql_is_deterministic_and_sized() {
        let a = r#gen(50_000, None);
        assert_eq!(a, r#gen(50_000, None));
        assert_eq!(a.lines().count(), 50_000);
        assert!((1_800_000..=2_200_000).contains(&a.len()));
    }

    #[test]
    fn gen_sql_long_line_has_exact_char_count() {
        let a = r#gen(100, Some(100_000));
        let last = a.lines().last().unwrap();
        assert_eq!(a.lines().count(), 101);
        assert_eq!(last.chars().count(), 100_000);
        assert!(a.contains("'İstanbul'") && a.contains("'Iğdır'"));
    }
}
