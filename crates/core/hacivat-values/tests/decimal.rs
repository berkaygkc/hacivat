//! M3a Task 6: exact decimals (HV-RES-01): text and PostgreSQL base-10000 in, canonical text
//! out, no binary float and no 128-bit integer on the path.

use hacivat_values::decimal::{MAX_INT_DIGITS, MAX_SCALE};
use hacivat_values::{DecimalError, DecimalRef, DecimalVal, Repr};
use proptest::prelude::*;

fn canon(d: DecimalVal) -> String {
    let mut s = String::new();
    d.write_canonical(&mut s);
    s
}

fn source_of(path: &str) -> &'static str {
    match path {
        "src/decimal.rs" => include_str!("../src/decimal.rs"),
        other => panic!("no source fixture for {other}"),
    }
}

#[test]
fn decimals_round_trip_text_exactly() {
    for s in [
        "1.50",
        "-0.00",
        "12345678901234567890123456789012345678",
        "0.000001",
    ] {
        assert_eq!(canon(DecimalVal::parse(s).unwrap()), s);
    }
}

#[test]
fn decimal_over_38_digits_is_big() {
    assert!(matches!(
        DecimalVal::parse(&"9".repeat(39)).unwrap().repr(),
        Repr::Big(_)
    ));
}

#[test]
fn pg_base10000_vectors() {
    assert_eq!(
        canon(DecimalVal::from_base10000(false, 1, 2, &[1, 2345])),
        "12345.00"
    );
    assert_eq!(
        canon(DecimalVal::from_base10000(true, -1, 6, &[1200])),
        "-0.120000"
    );
}

#[test]
fn no_f64_on_the_decimal_path() {
    assert!(!source_of("src/decimal.rs").contains("f64"));
}

#[test]
fn no_128_bit_integers_on_the_decimal_path() {
    let src = source_of("src/decimal.rs");
    assert!(!src.contains("i128") && !src.contains("u128"));
}

#[test]
fn the_mantissa_words_keep_value_at_32_bytes_and_8_aligned() {
    // A 16-byte-aligned mantissa would make `Value` 48 bytes (P8). The alignment is the
    // invariant; the sizes are budgets (a later layout may shrink them).
    assert_eq!(align_of::<DecimalVal>(), 8);
    assert!(size_of::<DecimalVal>() <= 24);
    assert_eq!(align_of::<hacivat_values::Value>(), 8);
    assert!(size_of::<hacivat_values::Value>() <= 32);
}

#[test]
fn value_ref_carries_its_decimal_by_value_in_32_bytes() {
    assert!(size_of::<DecimalRef<'_>>() <= 24);
    assert!(size_of::<hacivat_values::ValueRef<'_>>() <= 32);
}

/// Every way a `DecimalRef` reads back must be the `DecimalVal` it came from.
fn assert_ref_round_trips(d: &DecimalVal) {
    let r = d.view();
    assert_eq!(r.to_decimal(), *d);
    assert_eq!(r.scale(), d.scale());
    assert_eq!(r.is_negative(), d.is_negative());
    assert_eq!(r.is_big(), d.is_big());
    assert_eq!(r.to_string(), d.to_string());
    let mut text = String::new();
    r.write_canonical(&mut text);
    assert_eq!(text, canon(d.clone()));
    assert_eq!(DecimalRef::from(d), r);
    // Through `Value`: view, then back to an owned value.
    let v = hacivat_values::Value::Decimal(d.clone());
    assert_eq!(v.view().to_value(), v);
    assert_eq!(v.view(), hacivat_values::ValueRef::Decimal(r));
}

#[test]
fn decimal_ref_round_trips_scale_sign_and_text() {
    for s in [
        "1.50", "-1.50", "1.5", "-0.00", "0", "-0", "0.000001", "007.50",
    ] {
        assert_ref_round_trips(&DecimalVal::parse(s).unwrap());
    }
    let r = DecimalVal::parse("-0.00").unwrap();
    assert!(r.view().is_negative());
    assert_eq!(r.view().scale(), 2);
    assert_ne!(r.view(), DecimalVal::parse("0.00").unwrap().view());
    assert_ne!(
        DecimalVal::parse("1.50").unwrap().view(),
        DecimalVal::parse("1.5").unwrap().view()
    );
}

#[test]
fn decimal_ref_keeps_the_38_digit_maximum_and_big_text() {
    for s in [
        "9".repeat(38),
        format!("-{}", "9".repeat(38)),
        format!("0.{}", "9".repeat(38)),
    ] {
        let d = DecimalVal::parse(&s).unwrap();
        assert!(!d.view().is_big(), "{s}");
        assert_eq!(d.view().to_string(), s);
        assert_ref_round_trips(&d);
    }
    for s in [
        "9".repeat(39),
        format!("-{}.{}", "1".repeat(30), "2".repeat(30)),
    ] {
        let d = DecimalVal::parse(&s).unwrap();
        assert!(d.view().is_big(), "{s}");
        assert_eq!(d.view().to_string(), s);
        assert_ref_round_trips(&d);
    }
}

#[test]
fn decimal_ref_round_trips_try_from_base10000() {
    for (neg, weight, dscale, digits, text) in [
        (false, 1, 2, &[1u16, 2345][..], "12345.00".to_string()),
        (true, -1, 6, &[1200][..], "-0.120000".to_string()),
        (false, 0, 3, &[][..], "0.000".to_string()),
        (false, 9, 0, &[9_999; 10][..], "9".repeat(40)),
        (
            true,
            8,
            2,
            &[9_999; 9][..],
            format!("-{}.00", "9".repeat(36)),
        ),
    ] {
        let d = DecimalVal::try_from_base10000(neg, weight, dscale, digits).unwrap();
        assert_eq!(d.view().to_string(), text);
        assert_eq!(d.view().scale(), dscale as i16);
        assert_eq!(d.view().is_negative(), neg);
        assert_ref_round_trips(&d);
    }
}

#[test]
fn the_servers_scale_and_sign_are_kept() {
    let a = DecimalVal::parse("1.50").unwrap();
    let b = DecimalVal::parse("1.5").unwrap();
    assert_ne!(a, b);
    assert_eq!((a.scale(), b.scale()), (2, 1));
    let neg_zero = DecimalVal::parse("-0.00").unwrap();
    assert!(neg_zero.is_negative());
    assert_ne!(neg_zero, DecimalVal::parse("0.00").unwrap());
    assert_eq!(neg_zero.to_string(), "-0.00");
}

#[test]
fn small_layout_is_sign_and_magnitude_low_word_first() {
    // 2^64 + 5 = 18446744073709551621, negative, scale 1.
    let d = DecimalVal::parse("-1844674407370955162.1").unwrap();
    assert_eq!(
        d.repr(),
        &Repr::Small {
            mant: [5, 1 | 1 << 63],
            scale: 1
        }
    );
}

#[test]
fn thirty_eight_nines_are_small_thirty_nine_are_big() {
    let max = "9".repeat(38);
    assert!(matches!(
        DecimalVal::parse(&max).unwrap().repr(),
        Repr::Small { .. }
    ));
    assert_eq!(canon(DecimalVal::parse(&max).unwrap()), max);
    let big = format!("-{}.{}", "9".repeat(20), "1".repeat(19));
    let d = DecimalVal::parse(&big).unwrap();
    assert!(d.is_big());
    assert_eq!(d.scale(), 19);
    assert!(d.is_negative());
    assert_eq!(canon(d), big);
}

#[test]
fn leading_zeros_do_not_count_as_digits() {
    // 40 places, one significant digit: still Small.
    let s = format!("0.{}1", "0".repeat(39));
    let d = DecimalVal::parse(&s).unwrap();
    assert!(matches!(d.repr(), Repr::Small { scale: 40, .. }));
    assert_eq!(canon(d), s);
    assert_eq!(canon(DecimalVal::parse("007.50").unwrap()), "7.50");
    assert_eq!(canon(DecimalVal::parse("+.5").unwrap()), "0.5");
    assert_eq!(canon(DecimalVal::parse("5.").unwrap()), "5");
    assert_eq!(canon(DecimalVal::parse("-0").unwrap()), "-0");
}

#[test]
fn bad_text_is_refused() {
    assert_eq!(DecimalVal::parse(""), Err(DecimalError::Empty));
    for s in [
        "-", "+", ".", "-.", "1e5", " 1", "1 ", "1,5", "NaN", "Infinity", "1.2.3", "--1", "١",
    ] {
        assert_eq!(DecimalVal::parse(s), Err(DecimalError::Invalid), "{s:?}");
    }
    let wide = "1".repeat(MAX_INT_DIGITS + 1);
    assert_eq!(DecimalVal::parse(&wide), Err(DecimalError::TooLong));
    let deep = format!("0.{}", "1".repeat(MAX_SCALE as usize + 1));
    assert_eq!(DecimalVal::parse(&deep), Err(DecimalError::TooLong));
    // Exactly at the limits: accepted, held as text.
    assert!(
        DecimalVal::parse(&"1".repeat(MAX_INT_DIGITS))
            .unwrap()
            .is_big()
    );
}

#[test]
fn base10000_edge_vectors() {
    let b = |neg, weight, dscale, digits: &[u16]| {
        canon(DecimalVal::from_base10000(neg, weight, dscale, digits))
    };
    // Zero: no digits.
    assert_eq!(b(false, 0, 0, &[]), "0");
    assert_eq!(b(false, 0, 3, &[]), "0.000");
    // A weight above the digits: trailing zero groups.
    assert_eq!(b(false, 2, 0, &[7]), "700000000");
    // A negative weight: leading zero groups after the point.
    assert_eq!(b(false, -2, 8, &[42]), "0.00000042");
    // Digits ending exactly at dscale.
    assert_eq!(b(true, 0, 4, &[3, 1416]), "-3.1416");
    // More than 38 digits: Big, canonical.
    let d = DecimalVal::from_base10000(false, 9, 0, &[9_999; 10]);
    assert!(d.is_big());
    assert_eq!(canon(d), "9".repeat(40));
    // A long one that needs the heap.
    let d = DecimalVal::from_base10000(false, 40, 2, &[1]);
    assert_eq!(canon(d), format!("1{}.00", "0".repeat(160)));
}

#[test]
fn base10000_refuses_what_postgresql_would_not_send() {
    let t = DecimalVal::try_from_base10000;
    assert_eq!(t(false, 0, 0, &[10_000]), Err(DecimalError::Invalid));
    assert_eq!(t(false, 0, MAX_SCALE + 1, &[1]), Err(DecimalError::Invalid));
    // A nonzero digit beyond dscale would be dropped: refused, never rounded.
    assert_eq!(t(false, -1, 2, &[1234]), Err(DecimalError::Invalid));
    // Zeros beyond dscale are fine.
    assert_eq!(canon(t(false, -1, 2, &[1200]).unwrap()), "0.12");
    // Extreme weights stay bounded.
    assert!(t(false, i16::MAX, 0, &[1]).unwrap().is_big());
    assert_eq!(canon(t(false, i16::MIN, 0, &[0]).unwrap()), "0");
    assert_eq!(t(false, i16::MIN, 0, &[1]), Err(DecimalError::Invalid));
}

#[test]
#[should_panic(expected = "invalid base-10000 numeric")]
fn base10000_panics_only_through_the_unchecked_form() {
    DecimalVal::from_base10000(false, 0, 0, &[10_000]);
}

#[test]
fn from_mantissa_is_exact_at_the_i64_extremes() {
    assert_eq!(
        canon(DecimalVal::from_mantissa(i64::MIN, 2)),
        "-92233720368547758.08"
    );
    assert_eq!(
        canon(DecimalVal::from_mantissa(i64::MAX, 0)),
        "9223372036854775807"
    );
    assert_eq!(canon(DecimalVal::from_mantissa(15, 4)), "0.0015");
    assert_eq!(canon(DecimalVal::from_mantissa(15, -3)), "15000");
    assert_eq!(canon(DecimalVal::from_mantissa(0, -3)), "0");
}

/// PostgreSQL's base-10000 form of canonical decimal text, as `numeric_send` builds it:
/// groups of four aligned on the point, leading and trailing zero groups dropped.
fn to_base10000(text: &str) -> (bool, i16, u16, Vec<u16>) {
    let (neg, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let int_pad = (4 - int.len() % 4) % 4;
    let frac_pad = (4 - frac.len() % 4) % 4;
    let digits: String = "0".repeat(int_pad) + int + frac + &"0".repeat(frac_pad);
    let mut groups: Vec<u16> = digits
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap().parse().unwrap())
        .collect();
    let mut weight = ((int.len() + int_pad) / 4) as i16 - 1;
    while groups.first() == Some(&0) {
        groups.remove(0);
        weight -= 1;
    }
    while groups.last() == Some(&0) {
        groups.pop();
    }
    if groups.is_empty() {
        weight = 0;
    }
    (neg, weight, frac.len() as u16, groups)
}

fn canonical_text() -> impl Strategy<Value = String> {
    (
        any::<bool>(),
        prop_oneof![Just(String::from("0")), "[1-9][0-9]{0,45}"],
        prop::option::of("[0-9]{1,45}"),
    )
        .prop_map(|(neg, int, frac)| {
            let mut s = String::new();
            if neg {
                s.push('-');
            }
            s.push_str(&int);
            if let Some(f) = frac {
                s.push('.');
                s.push_str(&f);
            }
            s
        })
}

proptest! {
    #[test]
    fn canonical_text_round_trips(s in canonical_text()) {
        let d = DecimalVal::parse(&s).unwrap();
        prop_assert_eq!(canon(d.clone()), s.clone());
        let places = s.split_once('.').map_or(0, |(_, f)| f.len());
        prop_assert_eq!(d.scale() as usize, places);
        prop_assert_eq!(d.is_negative(), s.starts_with('-'));
        // And through `DecimalRef`, by value.
        prop_assert_eq!(d.view().to_decimal(), d.clone());
        prop_assert_eq!(d.view().to_string(), s.clone());
        prop_assert_eq!(d.view().scale(), d.scale());
        prop_assert_eq!(d.view().is_negative(), d.is_negative());
    }

    #[test]
    fn base10000_agrees_with_text(s in canonical_text()) {
        let (neg, weight, dscale, groups) = to_base10000(&s);
        prop_assert_eq!(
            DecimalVal::from_base10000(neg, weight, dscale, &groups),
            DecimalVal::parse(&s).unwrap()
        );
    }

    #[test]
    fn from_mantissa_agrees_with_text(m in any::<i64>(), scale in 0i16..30) {
        let mag = m.unsigned_abs().to_string();
        let scale_u = scale as usize;
        let padded = format!("{mag:0>width$}", width = scale_u + 1);
        let (int, frac) = padded.split_at(padded.len() - scale_u);
        let text = format!(
            "{}{int}{}{frac}",
            if m < 0 { "-" } else { "" },
            if scale_u > 0 { "." } else { "" }
        );
        prop_assert_eq!(DecimalVal::from_mantissa(m, scale), DecimalVal::parse(&text).unwrap());
    }

    /// Mostly valid digits and scales (so the layout code runs), with the extreme weights and
    /// an occasional invalid digit or scale; an accepted value must be its own canonical text.
    #[test]
    fn hostile_base10000_never_panics(
        neg in any::<bool>(),
        weight in prop_oneof![
            8 => -40i16..40,
            1 => any::<i16>(),
            1 => prop_oneof![Just(i16::MIN), Just(i16::MAX)],
        ],
        dscale in prop_oneof![9 => 0u16..=MAX_SCALE + 8, 1 => any::<u16>()],
        digits in prop::collection::vec(
            prop_oneof![20 => 0u16..10_000, 1 => any::<u16>()],
            0..40,
        ),
    ) {
        let valid_input = dscale <= MAX_SCALE && digits.iter().all(|&g| g < 10_000);
        match DecimalVal::try_from_base10000(neg, weight, dscale, &digits) {
            Ok(d) => {
                prop_assert!(valid_input);
                prop_assert_eq!(d.scale(), dscale as i16);
                prop_assert_eq!(DecimalVal::parse(&d.to_string()), Ok(d.clone()));
                prop_assert_eq!(d.view().to_decimal(), d);
            }
            Err(e) => prop_assert_eq!(e, DecimalError::Invalid),
        }
    }
}
