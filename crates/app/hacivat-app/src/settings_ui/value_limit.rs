//! ## m3b-t42-settings: the "Largest value kept per cell" row (`results.max_value_bytes`,
//! R-M3-82; NOTES surface 25): a number in the regional format and a KB / MB unit. The stored
//! value is the number times the unit (1,024-based; shown as "KB" and "MB", ruling M3c-32),
//! clamped to 64 KB–64 MB ([`ValueLimit`]); a typed size outside that range is set to the
//! bound and the row says so.
//!
//! The row edits a pair: choosing the other unit applies the field's number in that unit
//! (`1` with MB, then KB, is 1 KB, set to 64 KB with the note), so what the row shows is always
//! what is stored.

use hacivat_l10n::{ArgValue, ExactDecimal, MsgRef, NoArgs, Text, ids};
use hacivat_settings::ValueLimit;
use hacivat_ui_core::gpui_surface::{App, SharedString};

use crate::l10n_global::{L10nGlobal, tr};

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;

/// Most digits a typed number may have; more is "too large" (always above the limit).
const MAX_DIGITS: u32 = 30;
/// Fraction digits the field shows a size that is not a whole number of its unit with.
const SHOWN_FRACTION_DIGITS: u32 = 2;

/// The unit of the largest-value field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeUnit {
    Kb,
    Mb,
}

impl SizeUnit {
    /// In the select's order.
    pub const ALL: [SizeUnit; 2] = [SizeUnit::Kb, SizeUnit::Mb];

    pub fn bytes(self) -> u64 {
        match self {
            SizeUnit::Kb => KIB,
            SizeUnit::Mb => MIB,
        }
    }

    /// The select's label.
    pub fn label(self) -> MsgRef<NoArgs> {
        match self {
            SizeUnit::Kb => ids::hv::SETTINGS_UNIT_KB,
            SizeUnit::Mb => ids::hv::SETTINGS_UNIT_MB,
        }
    }

    /// The position in [`SizeUnit::ALL`].
    pub fn index(self) -> usize {
        match self {
            SizeUnit::Kb => 0,
            SizeUnit::Mb => 1,
        }
    }

    /// The `exec-size-*` message that writes a size in this unit.
    fn size_message(self) -> &'static str {
        match self {
            SizeUnit::Kb => "exec-size-kb",
            SizeUnit::Mb => "exec-size-mb",
        }
    }
}

/// The unit `bytes` is shown in: `preferred` while it measures `bytes` whole, else MB for a
/// whole number of megabytes and KB otherwise.
pub fn display_unit(bytes: u32, preferred: Option<SizeUnit>) -> SizeUnit {
    let whole = |unit: SizeUnit| u64::from(bytes) % unit.bytes() == 0;
    match preferred {
        Some(unit) if whole(unit) => unit,
        _ if whole(SizeUnit::Mb) => SizeUnit::Mb,
        _ => SizeUnit::Kb,
    }
}

/// A non-negative decimal as typed: `mantissa / 10^scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Amount {
    pub mantissa: u128,
    pub scale: u32,
}

impl Amount {
    /// The amount as exact decimal text (`"0.5"`, `"128"`), without trailing fraction zeros.
    pub fn decimal_text(self) -> String {
        let pow = 10u128.pow(self.scale);
        let whole = self.mantissa / pow;
        let width = usize::try_from(self.scale).unwrap_or(0);
        let fraction = format!("{:0width$}", self.mantissa % pow);
        let fraction = fraction.trim_end_matches('0');
        if fraction.is_empty() {
            whole.to_string()
        } else {
            format!("{whole}.{fraction}")
        }
    }

    /// The amount in bytes of `unit`, rounded half up (saturating).
    pub fn bytes(self, unit: SizeUnit) -> u128 {
        let pow = 10u128.pow(self.scale);
        self.mantissa
            .saturating_mul(u128::from(unit.bytes()))
            .saturating_add(pow / 2)
            / pow
    }

    /// `bytes` in `unit`, with at most [`SHOWN_FRACTION_DIGITS`] fraction digits (rounded half
    /// up).
    pub fn of(bytes: u32, unit: SizeUnit) -> Amount {
        let scale = SHOWN_FRACTION_DIGITS;
        let pow = 10u128.pow(scale);
        let unit = u128::from(unit.bytes());
        Amount {
            mantissa: (u128::from(bytes) * pow + unit / 2) / unit,
            scale,
        }
    }
}

/// Parses what was typed with the format locale's `decimal` and `group` separators: digits, the
/// group separator between digits of the whole part (a space-like `group` accepts any space),
/// then at most one `decimal` separator followed by digits. Anything else (a sign, a letter, a
/// unit) is not a size.
pub fn parse_amount(text: &str, decimal: char, group: char) -> Option<Amount> {
    let text = text.trim();
    let is_group = |c: char| c == group || (group.is_whitespace() && c.is_whitespace());
    let mut mantissa: u128 = 0;
    let mut digits = 0u32;
    let mut scale: Option<u32> = None;
    let mut last_group = false;
    for c in text.chars() {
        if let Some(d) = c.to_digit(10) {
            if digits < MAX_DIGITS {
                mantissa = mantissa * 10 + u128::from(d);
                digits += 1;
                if let Some(s) = scale.as_mut() {
                    *s += 1;
                }
            } else if scale.is_none() {
                // Too large: keep the size above every limit.
                return Some(Amount {
                    mantissa: 10u128.pow(MAX_DIGITS),
                    scale: 0,
                });
            }
            last_group = false;
        } else if c == decimal && scale.is_none() && digits > 0 && !last_group {
            scale = Some(0);
        } else if is_group(c) && c != decimal && scale.is_none() && digits > 0 && !last_group {
            last_group = true;
        } else {
            return None;
        }
    }
    if digits == 0 || last_group || scale == Some(0) {
        return None;
    }
    Some(Amount {
        mantissa,
        scale: scale.unwrap_or(0),
    })
}

/// Where a typed size fell against the allowed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fit {
    Within,
    /// Above 64 MB: set to 64 MB.
    Above,
    /// Below 64 KB: set to 64 KB.
    Below,
}

/// The limit a size of `bytes` sets, and whether it had to be clamped.
pub fn fit(bytes: u128) -> (ValueLimit, Fit) {
    let limit = ValueLimit::clamped(i64::try_from(bytes).unwrap_or(i64::MAX));
    let fit = if bytes > u128::from(ValueLimit::MAX) {
        Fit::Above
    } else if bytes < u128::from(ValueLimit::MIN) {
        Fit::Below
    } else {
        Fit::Within
    };
    (limit, fit)
}

/// The note under the field after a typed size was clamped: what was typed, in its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Clamped {
    pub typed: Amount,
    pub unit: SizeUnit,
    pub fit: Fit,
}

/// The field's number for `amount`, in the regional format (`1.536` in Turkey).
pub fn amount_text(cx: &App, amount: Amount) -> String {
    let rendered = tr(
        cx,
        ids::hv::SETTINGS_MAX_VALUE_AMOUNT,
        ids::hv::HvSettingsMaxValueAmount {
            n: ExactDecimal(amount.decimal_text()),
        },
    );
    number_part(&rendered).to_string()
}

/// The number inside a rendered message: from its first digit to its last (the isolation
/// marks and a pseudo-locale's brackets are not part of the field).
fn number_part(rendered: &str) -> &str {
    let first = rendered.find(|c: char| c.is_ascii_digit());
    let last = rendered
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, c)| i + c.len_utf8());
    match (first, last) {
        (Some(a), Some(b)) => &rendered[a..b],
        _ => "",
    }
}

/// The format locale's decimal and group separators, as the field writes 1,234.5.
pub fn separators(cx: &App) -> (char, char) {
    let sample = amount_text(
        cx,
        Amount {
            mantissa: 12_345,
            scale: 1,
        },
    );
    let marks: Vec<char> = sample.chars().filter(|c| !c.is_ascii_digit()).collect();
    match marks.as_slice() {
        [group, .., decimal] => (*decimal, *group),
        [decimal] => (*decimal, ','),
        [] => ('.', ','),
    }
}

/// A size as the `exec-size-*` messages write it ("128 MB"), its number in the regional format.
fn size_text(cx: &App, amount: Amount, unit: SizeUnit) -> String {
    let text = amount.decimal_text();
    let n = match text.parse::<u64>() {
        Ok(whole) => ArgValue::Count(whole),
        Err(_) => ArgValue::Decimal(ExactDecimal(text)),
    };
    cx.try_global::<L10nGlobal>().map_or_else(
        || unit.size_message().to_string(),
        |g| g.l10n.t_dynamic(unit.size_message(), vec![("n", n)]),
    )
}

/// The clamp note's sentence.
pub fn clamped_text(cx: &App, clamped: Clamped) -> SharedString {
    let typed = Text(size_text(cx, clamped.typed, clamped.unit));
    let bound = |bytes: u32| {
        let unit = display_unit(bytes, None);
        Text(size_text(cx, Amount::of(bytes, unit), unit))
    };
    match clamped.fit {
        Fit::Below => tr(
            cx,
            ids::hv::SETTINGS_MAX_VALUE_CLAMPED_LOW,
            ids::hv::HvSettingsMaxValueClampedLow {
                typed,
                min: bound(ValueLimit::MIN),
            },
        ),
        Fit::Above | Fit::Within => tr(
            cx,
            ids::hv::SETTINGS_MAX_VALUE_CLAMPED,
            ids::hv::HvSettingsMaxValueClamped {
                typed,
                max: bound(ValueLimit::MAX),
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(mantissa: u128, scale: u32) -> Amount {
        Amount { mantissa, scale }
    }

    #[test]
    fn typed_sizes_parse_with_the_locale_separators() {
        assert_eq!(parse_amount("128", '.', ','), Some(amount(128, 0)));
        assert_eq!(parse_amount(" 1,024 ", '.', ','), Some(amount(1024, 0)));
        assert_eq!(parse_amount("0.5", '.', ','), Some(amount(5, 1)));
        // Turkish: `.` groups, `,` is the decimal separator.
        assert_eq!(parse_amount("1.024", ',', '.'), Some(amount(1024, 0)));
        assert_eq!(parse_amount("1,5", ',', '.'), Some(amount(15, 1)));
        // A space-like group separator accepts any space.
        assert_eq!(
            parse_amount("65\u{a0}536", ',', '\u{202f}'),
            Some(amount(65_536, 0))
        );
        for bad in [
            "", "-1", "1e3", "1.", ",5", "1,,000", "12 MB", "1,000,", "1.5.5",
        ] {
            assert_eq!(parse_amount(bad, '.', ','), None, "{bad:?}");
        }
    }

    #[test]
    fn a_size_outside_the_range_is_set_to_the_bound() {
        assert_eq!(fit(65_536), (ValueLimit::new(65_536).unwrap(), Fit::Within));
        assert_eq!(fit(65_535).1, Fit::Below);
        assert_eq!(fit(65_535).0.get(), ValueLimit::MIN);
        assert_eq!(fit(64 << 20).1, Fit::Within);
        assert_eq!(
            fit((64 << 20) + 1),
            (ValueLimit::clamped(i64::MAX), Fit::Above)
        );
        assert_eq!(fit(0).0.get(), ValueLimit::MIN);
        assert_eq!(fit(u128::MAX).0.get(), ValueLimit::MAX);
        // 128 MB, 0.01 MB, 512 KB.
        assert_eq!(amount(128, 0).bytes(SizeUnit::Mb), 128 << 20);
        assert_eq!(amount(1, 2).bytes(SizeUnit::Mb), 10_486);
        assert_eq!(amount(512, 0).bytes(SizeUnit::Kb), 524_288);
        let huge = parse_amount(&"9".repeat(40), '.', ',').unwrap();
        assert_eq!(fit(huge.bytes(SizeUnit::Mb)).1, Fit::Above);
    }

    #[test]
    fn a_limit_shows_in_the_unit_it_is_whole_in() {
        assert_eq!(display_unit(1 << 20, None), SizeUnit::Mb);
        assert_eq!(display_unit(512 << 10, None), SizeUnit::Kb);
        assert_eq!(display_unit(1 << 20, Some(SizeUnit::Kb)), SizeUnit::Kb);
        assert_eq!(display_unit(512 << 10, Some(SizeUnit::Mb)), SizeUnit::Kb);
        assert_eq!(Amount::of(1536 << 10, SizeUnit::Kb).decimal_text(), "1536");
        assert_eq!(Amount::of(1536 << 10, SizeUnit::Mb).decimal_text(), "1.5");
        assert_eq!(Amount::of(100_000, SizeUnit::Kb).decimal_text(), "97.66");
        assert_eq!(number_part("[\u{2068}1.536\u{2069}]"), "1.536");
    }
}
