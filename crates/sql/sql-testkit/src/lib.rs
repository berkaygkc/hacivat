//! The SQL corpora (`corpus/`: lexing cases, split fixtures, the D-011 CARET/SEL/CHOOSE
//! goldens) and the golden loader and runner (Task 13). Clean room: fixtures are the
//! Hacivat-authored SQL of `docs/research/datagrip/13-behavioral-test-corpus.md` §3.3;
//! expectations come from Hacivat's specs, never from DataGrip (ALG-TEST-01).

pub mod golden;

// ## m3a-t04-lexer
/// The lexer corpus and the helpers lexer tests share (Task 4).
pub mod lex;

// ## m3a-t08-split
/// The splitter corpora and their checker (Task 8).
pub mod split;
