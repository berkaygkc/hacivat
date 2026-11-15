//! The D-011 goldens (Task 13): the CARET, SEL and CHOOSE families of
//! `docs/research/datagrip/13-behavioral-test-corpus.md`, with Hacivat's own expectations
//! (ALG-TEST-01, clean room), and the ID-coverage meta-test: no corpus ID is dropped silently.

use std::collections::BTreeMap;

use sql_testkit::golden::{self, Case, Outcome};

const FAMILIES: [(&str, &str, usize); 3] = [
    ("caret", "CARET", 30),
    ("sel", "SEL", 8),
    ("choose", "CHOOSE", 7),
];

/// Every skipped ID, with its reason: exactly these.
const EXPECTED_SKIPS: [(&str, &str); 8] = [
    ("CARET-19", "dialect:mssql"),
    ("CARET-20", "dialect:mssql"),
    ("CARET-22", "dialect:oracle"),
    ("SEL-06", "dialect:mssql"),
    ("CHOOSE-04", "dialect:mssql"),
    ("CHOOSE-05", "app-ui"),
    ("CHOOSE-06", "app-ui"),
    ("CHOOSE-07", "app-ui"),
];

/// Per family: (ran, ran without a CST, skipped).
const EXPECTED_COUNTS: [(&str, usize, usize, usize); 3] =
    [("CARET", 18, 9, 3), ("SEL", 7, 0, 1), ("CHOOSE", 3, 0, 4)];

fn cases(file: &str) -> Vec<Case> {
    golden::load(&golden::corpus(&format!("datagrip/{file}.toml")))
}

fn all_cases() -> Vec<Case> {
    FAMILIES
        .iter()
        .flat_map(|(file, _, _)| cases(file))
        .collect()
}

#[test]
fn caret_sel_choose_goldens() {
    let mut failures = Vec::new();
    let mut counts: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for case in all_cases() {
        let c = counts.entry(case.family().to_owned()).or_default();
        match golden::run(&case) {
            Ok(Outcome::Ran) => c.0 += 1,
            Ok(Outcome::RanWithoutCst) => c.1 += 1,
            Ok(Outcome::Skipped(_)) => c.2 += 1,
            Err(e) => failures.push(e),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let got: Vec<(&str, usize, usize, usize)> = counts
        .iter()
        .map(|(f, (ran, no_cst, skipped))| (f.as_str(), *ran, *no_cst, *skipped))
        .collect();
    let mut want = EXPECTED_COUNTS.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "(family, ran, ran without a CST, skipped)");
}

#[test]
fn every_corpus_id_is_present_or_skipped_with_a_reason() {
    let mut skips = Vec::new();
    for (file, family, count) in FAMILIES {
        // The ID lists are the research document's: `<FAMILY>-01` … `<FAMILY>-<count>`.
        let ids = golden::ids(file);
        let numbered: Vec<String> = (1..=count).map(|n| format!("{family}-{n:02}")).collect();
        assert_eq!(ids, numbered, "corpus/datagrip/ids/{file}.txt");

        let cases = cases(file);
        let present: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        let missing: Vec<&String> = ids
            .iter()
            .filter(|id| !present.contains(&id.as_str()))
            .collect();
        assert!(missing.is_empty(), "{file}.toml lacks {missing:?}");
        // No ID twice, and none the document does not have.
        assert_eq!(present.len(), ids.len(), "{file}.toml: {present:?}");
        for case in &cases {
            assert!(
                ids.contains(&case.id),
                "{file}.toml: unknown ID {}",
                case.id
            );
            assert!(!case.sources.is_empty(), "{}: no source", case.id);
            assert!(
                case.datagrip_observed.is_none(),
                "{}: no black-box observation was ever recorded",
                case.id
            );
            match golden::run(case) {
                Ok(Outcome::Skipped(reason)) => skips.push((case.id.clone(), reason)),
                Ok(_) => assert!(case.skip.is_none()),
                Err(e) => panic!("{e}"),
            }
        }
    }
    let want: Vec<(String, String)> = EXPECTED_SKIPS
        .iter()
        .map(|(id, reason)| ((*id).to_owned(), (*reason).to_owned()))
        .collect();
    assert_eq!(skips, want);
}

#[test]
fn a_requirement_the_runner_lacks_must_be_an_explicit_skip() {
    let mut case = cases("caret")
        .into_iter()
        .find(|c| c.id == "CARET-19")
        .unwrap();
    assert_eq!(
        golden::run(&case),
        Ok(Outcome::Skipped("dialect:mssql".into()))
    );
    // Without its `skip` the case is an error, never a silent pass.
    case.skip = None;
    assert!(golden::run(&case).unwrap_err().contains("dialect:mssql"));
    // A skip must name one of the case's requirements.
    case.skip = Some("because".into());
    assert!(golden::run(&case).is_err());
}

#[test]
fn a_wrong_expectation_fails_its_case() {
    let mut case = cases("caret")
        .into_iter()
        .find(|c| c.id == "CARET-07")
        .unwrap();
    assert_eq!(golden::run(&case), Ok(Outcome::Ran));
    // CARET-07 runs nothing: expecting a unit there must fail, naming the case.
    let expect = case.expect.as_mut().unwrap();
    expect.resolution = None;
    expect.units_text = Some(vec!["SELECT 2".into()]);
    let err = golden::run(&case).unwrap_err();
    assert!(err.starts_with("CARET-07: "), "{err}");
    assert!(err.contains("nothing to run"), "{err}");

    // A case without an expectation is malformed.
    case.expect = None;
    assert!(golden::run(&case).is_err());
}

#[test]
fn a_cst_case_runs_its_fallback_and_keeps_the_m5_answer() {
    for case in all_cases() {
        if case.requires.iter().any(|r| r == "cst") {
            assert!(case.expect.is_some(), "{}: no M5 expectation", case.id);
            assert!(
                case.expect_without_cst.is_some(),
                "{}: no M3a expectation",
                case.id
            );
            assert_eq!(
                golden::run(&case),
                Ok(Outcome::RanWithoutCst),
                "{}",
                case.id
            );
        }
    }
}

#[test]
fn the_divergence_register_names_real_cases() {
    let ids: Vec<String> = FAMILIES
        .iter()
        .flat_map(|(file, _, _)| golden::ids(file))
        .collect();
    let register = golden::divergences();
    for d in &register {
        assert!(ids.contains(&d.id), "{} is not a corpus ID", d.id);
        assert!(!d.what.is_empty() && !d.reason.is_empty() && !d.sources.is_empty());
    }
    // CHOOSE-01's missing subquery candidate (correlated; `alg/10` SQL-SPLIT-04).
    let choose = register.iter().find(|d| d.id == "CHOOSE-01").unwrap();
    assert!(choose.what.contains("no subquery candidate"));
    assert!(register.iter().any(|d| d.id == "SEL-07"));
}
