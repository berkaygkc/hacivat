//! Synthetic, storage-free data source for the grid spike.

use hacivat_ui_core::gpui_surface::SharedString;

pub struct ColumnMeta {
    pub name: SharedString,
    pub type_label: SharedString,
    pub width_px: f32,
    pub numeric: bool,
}

pub trait GridDataSource {
    fn row_count(&self) -> u64;
    fn col_count(&self) -> usize;
    fn column(&self, col: usize) -> ColumnMeta;
    /// Replaces `out` with the cell's display text. NULL cells write [`NULL`].
    fn write_cell(&self, row: u64, col: usize, out: &mut String);
}

/// The literal a source writes for a NULL cell; the grid renders it as a small-caps marker.
pub const NULL: &str = "NULL";

/// Deterministic cells computed from a hash of `(row, col)`; nothing is stored.
/// Column kinds cycle int, text, numeric(14,2), timestamp. Every 97th row of each column is
/// NULL (staggered by column, so no row is entirely NULL).
pub struct SyntheticSource {
    rows: u64,
    cols: usize,
}

impl SyntheticSource {
    pub fn new(rows: u64, cols: usize) -> Self {
        Self { rows, cols }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int,
    Text,
    Numeric,
    Timestamp,
}

fn kind_of(col: usize) -> Kind {
    match col % 4 {
        0 => Kind::Int,
        1 => Kind::Text,
        2 => Kind::Numeric,
        _ => Kind::Timestamp,
    }
}

fn is_null(row: u64, col: usize) -> bool {
    (row + col as u64) % 97 == 96
}

/// SplitMix64 finalizer over `(row, col)`.
fn cell_hash(row: u64, col: usize) -> u64 {
    let mut z = row
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((col as u64).wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(0x632B_E59B_D9B4_E019);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Mixed ASCII and Turkish words, so shaping sees non-ASCII glyphs too.
const WORDS: [&str; 16] = [
    "Ankara",
    "İstanbul",
    "Çankaya",
    "Kadıköy",
    "Ümraniye",
    "Şişli",
    "invoice",
    "refund",
    "pending",
    "shipped",
    "customer",
    "warehouse",
    "Göztepe",
    "Beşiktaş",
    "priority",
    "archive",
];

/// Appends `v` with en-US thousands separators.
fn push_grouped(out: &mut String, v: u64) {
    let mut buf = [0u8; 20];
    let mut n = v;
    let mut len = 0;
    loop {
        buf[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for i in (0..len).rev() {
        out.push(buf[i] as char);
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
    }
}

/// Days since 1970-01-01 to a civil `(year, month, day)` (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

impl GridDataSource for SyntheticSource {
    fn row_count(&self) -> u64 {
        self.rows
    }

    fn col_count(&self) -> usize {
        self.cols
    }

    fn column(&self, col: usize) -> ColumnMeta {
        let (name, type_label, width_px, numeric) = match kind_of(col) {
            Kind::Int => ("id", "int8", 104.0, true),
            Kind::Text => ("label", "text", 168.0, false),
            Kind::Numeric => ("amount", "numeric(14,2)", 148.0, true),
            Kind::Timestamp => ("created_at", "timestamp", 160.0, false),
        };
        ColumnMeta {
            name: format!("{name}_{col}").into(),
            type_label: type_label.into(),
            width_px,
            numeric,
        }
    }

    fn write_cell(&self, row: u64, col: usize, out: &mut String) {
        use std::fmt::Write as _;
        out.clear();
        if is_null(row, col) {
            out.push_str(NULL);
            return;
        }
        let h = cell_hash(row, col);
        match kind_of(col) {
            Kind::Int => {
                let _ = write!(out, "{}", (h >> 33) as i64 - (1 << 30));
            }
            Kind::Text => {
                out.push_str(WORDS[(h & 15) as usize]);
                if h & 0x30 != 0 {
                    out.push(' ');
                    out.push_str(WORDS[((h >> 8) & 15) as usize]);
                }
            }
            Kind::Numeric => {
                // numeric(14,2): up to 12 integer digits and 2 fractional digits. Mostly
                // everyday magnitudes, with one cell in eight using the full width.
                let full = (h >> 8) % 100_000_000_000_000;
                let cents = if h & 7 == 0 {
                    full
                } else {
                    full % 1_000_000_000
                };
                if h & 0x10 != 0 {
                    out.push('-');
                }
                push_grouped(out, cents / 100);
                let _ = write!(out, ".{:02}", cents % 100);
            }
            Kind::Timestamp => {
                // 2000-01-01 .. ~2030, second resolution.
                let secs = 946_684_800 + (h >> 20) % (30 * 365 * 86_400);
                let (y, m, d) = civil_from_days((secs / 86_400) as i64);
                let t = secs % 86_400;
                let _ = write!(
                    out,
                    "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
                    t / 3600,
                    (t / 60) % 60,
                    t % 60
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_cells_are_deterministic() {
        let s = SyntheticSource::new(10_000_000, 50);
        let (mut a, mut b) = (String::new(), String::new());
        s.write_cell(9_999_999, 49, &mut a);
        s.write_cell(9_999_999, 49, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn column_kinds_cycle_and_numerics_are_flagged() {
        let s = SyntheticSource::new(10_000_000, 50);
        let labels: Vec<_> = (0..5).map(|c| s.column(c).type_label.to_string()).collect();
        assert_eq!(
            labels,
            ["int8", "text", "numeric(14,2)", "timestamp", "int8"]
        );
        assert!(s.column(0).numeric && s.column(2).numeric);
        assert!(!s.column(1).numeric && !s.column(3).numeric);
    }

    #[test]
    fn every_97th_row_is_null() {
        let s = SyntheticSource::new(10_000_000, 50);
        let mut out = String::new();
        for col in [0, 1, 2, 3, 49] {
            let nulls = (0..97 * 100)
                .filter(|&row| {
                    s.write_cell(row, col, &mut out);
                    out == NULL
                })
                .count();
            assert_eq!(nulls, 100, "col {col}");
        }
    }

    #[test]
    fn thousands_grouping() {
        let g = |v| {
            let mut s = String::new();
            push_grouped(&mut s, v);
            s
        };
        assert_eq!(g(0), "0");
        assert_eq!(g(999), "999");
        assert_eq!(g(1_000), "1,000");
        assert_eq!(g(1_234_567), "1,234,567");
        assert_eq!(g(999_999_999_999), "999,999,999,999");
    }

    #[test]
    fn numeric_uses_thousands_separators_and_two_decimals() {
        let s = SyntheticSource::new(10_000_000, 50);
        let mut out = String::new();
        let mut saw_group = false;
        for row in 0..2_000 {
            s.write_cell(row, 2, &mut out);
            if out == NULL {
                continue;
            }
            let body = out.trim_start_matches('-');
            let (int, frac) = body.split_once('.').expect("decimal point");
            assert_eq!(frac.len(), 2, "{out}");
            let groups: Vec<_> = int.split(',').collect();
            assert!(
                groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3),
                "{out}"
            );
            assert!(int.replace(',', "").len() <= 12, "{out}");
            saw_group |= groups.len() > 1;
        }
        assert!(saw_group);
    }

    #[test]
    fn timestamps_are_well_formed() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let s = SyntheticSource::new(10_000_000, 50);
        let mut out = String::new();
        s.write_cell(1, 3, &mut out);
        let b = out.as_bytes();
        assert_eq!(out.len(), 19, "{out}");
        assert!(
            b[4] == b'-' && b[7] == b'-' && b[10] == b' ' && b[13] == b':',
            "{out}"
        );
    }

    #[test]
    fn cells_vary_across_rows() {
        let s = SyntheticSource::new(10_000_000, 50);
        let mut seen = std::collections::HashSet::new();
        let mut out = String::new();
        for row in 0..50 {
            s.write_cell(row, 0, &mut out);
            seen.insert(out.clone());
        }
        assert!(seen.len() > 40);
    }
}
