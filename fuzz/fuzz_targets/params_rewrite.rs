//! Fuzz target `params_rewrite` (SQL-T13; Task 20): the parameter scan (SQL-PARAM-01) of every
//! statement the execution split finds. A rewrite replaces exactly its bindable placeholders,
//! in order, copying every other byte; each replacement lexes as one placeholder of the
//! engine; scanning the rewritten text again rewrites nothing more. Nothing is rewritten when
//! a placeholder cannot bind. The seeded PR form is `sql-split/tests/params.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, TokenKind, lex};
use sql_split::{ParamSettings, SplitMode, SplitStart, StmtHead, scan_params, split_from};

fuzz_target!(|data: &[u8]| {
    let Some((&knobs, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let settings = ParamSettings {
        pg_question_marks: knobs & 1 != 0,
        colon_params: knobs & 2 == 0,
        in_ddl: knobs & 4 != 0,
    };
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        let ctx = DialectCtx::new(d, None);
        let Ok(profile) = lex_profile(&ctx) else {
            continue;
        };
        let split = split_from(
            text,
            &ctx,
            ClientFlavor::None,
            SplitMode::Execution,
            &SplitStart::default(),
        );
        let mut units: Vec<(&str, StmtHead)> = split
            .statements()
            .map(|s| (&text[s.strict], s.head))
            .collect();
        units.push((text, StmtHead::default()));
        for (unit, head) in units {
            let p = scan_params(unit, &ctx, &head, &settings);
            let Some(r) = &p.rewritten else {
                assert!(
                    p.placeholders.is_empty() || p.placeholders.iter().all(|x| !x.bindable),
                    "{d:?}: placeholders bind but nothing is rewritten"
                );
                continue;
            };
            let (mut src, mut dst) = (0usize, 0usize);
            for (from, to) in &r.edits {
                let (fs, fe) = (usize::from(from.start()), usize::from(from.end()));
                let (ts, te) = (usize::from(to.start()), usize::from(to.end()));
                assert!(fs >= src && ts >= dst, "{d:?}: edits out of order");
                assert_eq!(&unit[src..fs], &r.text[dst..ts], "{d:?}: a byte changed");
                assert!(
                    p.placeholders
                        .iter()
                        .any(|x| x.bindable && x.range == *from),
                    "{d:?}: an edit that is no bindable placeholder"
                );
                src = fe;
                dst = te;
            }
            assert_eq!(&unit[src..], &r.text[dst..], "{d:?}: the tail changed");
            let l = lex(&r.text, profile, ctx.modes, &LexState::Initial);
            for (_, to) in &r.edits {
                let i = l.token_at(to.start()).expect("a token at a replacement");
                assert_eq!(
                    l.range(i),
                    *to,
                    "{d:?}: a replacement lexes as more than a token"
                );
                assert!(
                    matches!(
                        l.kind(i),
                        TokenKind::PositionalParam | TokenKind::QuestionParam
                    ),
                    "{d:?}: a replacement is no placeholder"
                );
            }
            let again = scan_params(&r.text, &ctx, &head, &settings);
            assert_eq!(
                again.rewritten.map(|x| x.text),
                Some(r.text.clone()),
                "{d:?}: the rewritten text rewrites again"
            );
        }
    }
});
