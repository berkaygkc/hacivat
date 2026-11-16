//! M3a Task 17: column auto-size (HV-GRID-04): type default widths, the sampled measure through
//! a closure, the outlier trim, the header cap, the clamps and the fill.

use hacivat_ui_grid::ColumnKind;
use hacivat_ui_grid::model::autosize::{
    self, MAX_HEADER_WIDTH, MAX_INITIAL_WIDTH, MAX_WIDTH, MIN_WIDTH, Trimmed, WidthClass,
};

/// The initial width of a column whose header measures 60 px.
fn trim(widths: &[f32]) -> f32 {
    autosize::column_width(widths, 60.0)
}

#[test]
fn type_default_widths() {
    let cases = [
        (WidthClass::Bool, 64.0),
        (WidthClass::Integer, 96.0),
        (WidthClass::Decimal, 120.0),
        (WidthClass::Uuid, 300.0),
        (WidthClass::Timestamp, 200.0),
        (WidthClass::Text, 160.0),
        (WidthClass::Json, 200.0),
        (WidthClass::Binary, 200.0),
    ];
    for (class, width) in cases {
        assert_eq!(class.default_width(), width, "{class:?}");
    }
}

#[test]
fn outlier_trim() {
    assert_eq!(trim(&[40.0, 42.0, 45.0, 41.0, 300.0, 44.0]), 60.0);
    assert_eq!(
        autosize::trim_outliers(&[40.0, 42.0, 45.0, 41.0, 300.0, 44.0]),
        Trimmed {
            width: 45.0,
            trimmed: 1
        }
    );
    // Five samples or fewer are never trimmed.
    assert_eq!(autosize::column_width(&[40.0, 42.0, 300.0], 0.0), 300.0);
    assert_eq!(
        autosize::trim_outliers(&[40.0, 42.0, 300.0]),
        Trimmed {
            width: 300.0,
            trimmed: 0
        }
    );
}

#[test]
fn up_to_three_passes_from_twenty_samples() {
    // Each pass drops one more outlier: 10,000, then 600, then 50; a fourth would drop 21.
    let mut many = vec![10.0; 30];
    many.extend([10_000.0, 600.0, 50.0, 21.0]);
    assert_eq!(
        autosize::trim_outliers(&many),
        Trimmed {
            width: 21.0,
            trimmed: 3
        }
    );
    // Below twenty samples there is one pass: 150 survives the first mean.
    let mut few = vec![10.0; 10];
    few.extend([1_000.0, 150.0]);
    assert_eq!(
        autosize::trim_outliers(&few),
        Trimmed {
            width: 150.0,
            trimmed: 1
        }
    );
}

#[test]
fn degenerate_samples() {
    assert_eq!(
        autosize::trim_outliers(&[]),
        Trimmed {
            width: 0.0,
            trimmed: 0
        }
    );
    // Empty strings (width 0) are not all outliers of a zero mean.
    assert_eq!(autosize::trim_outliers(&[0.0; 30]).width, 0.0);
    // Non-finite measurements are ignored.
    assert_eq!(
        autosize::trim_outliers(&[f32::NAN, 70.0, f32::INFINITY, -1.0]).width,
        70.0
    );
    // An empty result is sized by its header.
    assert_eq!(autosize::column_width(&[], 70.0), 70.0);
}

#[test]
fn all_null_column_with_a_short_header_takes_the_minimum() {
    assert_eq!(autosize::column_width(&[30.0; 80], 10.0), MIN_WIDTH);
    assert_eq!(MIN_WIDTH, 48.0);
}

#[test]
fn header_width_is_capped_at_320() {
    assert_eq!(MAX_HEADER_WIDTH, 320.0);
    assert_eq!(autosize::column_width(&[100.0; 10], 500.0), 320.0);
    assert_eq!(autosize::column_width(&[100.0; 10], 60.0), 100.0);
    // Values wider than the cap still win over a capped header.
    assert_eq!(autosize::column_width(&[400.0; 10], 500.0), 400.0);
}

#[test]
fn clamps_initial_fit_and_user_widths() {
    assert_eq!((MAX_INITIAL_WIDTH, MAX_WIDTH), (480.0, 2_000.0));
    assert_eq!(autosize::column_width(&[1_000.0; 10], 50.0), 480.0);
    assert_eq!(autosize::fit_width(&[1_000.0; 10], 50.0), 1_000.0);
    assert_eq!(autosize::fit_width(&[5_000.0; 10], 50.0), 2_000.0);
    assert_eq!(autosize::fit_width(&[], 10.0), 48.0);
    assert_eq!(autosize::clamp_user_width(10.0), 48.0);
    assert_eq!(autosize::clamp_user_width(3_000.0), 2_000.0);
    assert_eq!(autosize::clamp_user_width(f32::NAN), 48.0);
    assert_eq!(autosize::clamp_user_width(250.5), 250.5);
    // Initial widths snap up to whole pixels.
    assert_eq!(autosize::column_width(&[100.2; 3], 0.0), 101.0);
}

#[test]
fn samples_are_head_tail_and_strata() {
    let s = autosize::sample_rows(1_000_000);
    assert_eq!(s.len(), 80);
    assert_eq!(&s[..50], &(0..50).collect::<Vec<u64>>()[..]);
    assert_eq!(&s[70..], &(999_990..1_000_000).collect::<Vec<u64>>()[..]);
    let strata = &s[50..70];
    assert!(strata.windows(2).all(|w| w[0] < w[1]), "{strata:?}");
    assert!(strata.iter().all(|r| (50..999_990).contains(r)));
    // One sample in each twentieth of the middle.
    let span = (999_990 - 50) as f64 / 20.0;
    for (i, r) in strata.iter().enumerate() {
        let k = ((*r - 50) as f64 / span) as usize;
        assert_eq!(k, i, "row {r}");
    }

    assert_eq!(autosize::sample_rows(70), (0..70).collect::<Vec<u64>>());
    assert!(autosize::sample_rows(0).is_empty());
    let s = autosize::sample_rows(81);
    assert_eq!(s.len(), 80);
    assert!(s.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(autosize::sample_rows(u64::MAX).len(), 80);
}

#[test]
fn the_measure_runs_only_over_the_samples() {
    let mut calls = Vec::new();
    let w = autosize::initial_width(1_000_000, 30.0, |row| {
        calls.push(row);
        if row < 50 { 70.0 } else { 90.0 }
    });
    assert_eq!(w, 90.0);
    assert_eq!(calls, autosize::sample_rows(1_000_000));
    // No rows: the header alone, clamped.
    assert_eq!(autosize::initial_width(0, 30.0, |_| unreachable!()), 48.0);
}

#[test]
fn fill_grows_text_like_columns_and_never_numbers() {
    let mut w = [96.0, 160.0, 160.0];
    autosize::fill(&mut w, &[false, true, true], 960.0);
    assert_eq!(w, [96.0, 432.0, 432.0]);

    let mut w = [96.0, 100.0, 300.0];
    autosize::fill(&mut w, &[false, true, true], 1_000.0);
    assert_eq!(w, [96.0, 226.0, 678.0]);

    // Whole pixels; the remainder goes to the last text-like column.
    let mut w = [50.0, 100.0, 100.0, 100.0];
    autosize::fill(&mut w, &[false, true, true, true], 351.0);
    assert_eq!(w, [50.0, 100.0, 100.0, 101.0]);
    assert_eq!(w.iter().sum::<f32>(), 351.0);

    // Never past the absolute maximum.
    let mut w = [100.0];
    autosize::fill(&mut w, &[true], 5_000.0);
    assert_eq!(w, [2_000.0]);
}

#[test]
fn fill_applies_only_to_narrow_results_that_do_not_fill_the_viewport() {
    let mut nine = [100.0; 9];
    autosize::fill(&mut nine, &[true; 9], 5_000.0);
    assert_eq!(nine, [100.0; 9], "more than eight columns");

    let mut wide = [600.0, 600.0];
    autosize::fill(&mut wide, &[true, true], 1_000.0);
    assert_eq!(wide, [600.0, 600.0], "already wider than the viewport");

    let mut numbers = [96.0, 120.0];
    autosize::fill(&mut numbers, &[false, false], 1_000.0);
    assert_eq!(numbers, [96.0, 120.0], "no text-like column");

    let mut odd = [96.0, 120.0];
    autosize::fill(&mut odd, &[true], 1_000.0);
    assert_eq!(odd, [96.0, 120.0], "mismatched flags change nothing");

    let mut w = [96.0, 120.0];
    autosize::fill(&mut w, &[true, true], f32::NAN);
    assert_eq!(w, [96.0, 120.0]);
}

#[test]
fn text_like_kinds() {
    for kind in [ColumnKind::Text, ColumnKind::Json, ColumnKind::Other] {
        assert!(autosize::is_text_like(kind), "{kind:?}");
    }
    for kind in [
        ColumnKind::Number,
        ColumnKind::Bool,
        ColumnKind::Temporal,
        ColumnKind::Uuid,
        ColumnKind::Binary,
    ] {
        assert!(!autosize::is_text_like(kind), "{kind:?}");
    }
}
