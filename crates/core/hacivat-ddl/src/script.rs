//! A generated script: statements plus the notices shown next to them.

use crate::error::Notice;
use crate::render::{Delim, Stmt};

/// How statements end in the text form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DelimStyle {
    /// Every statement ends with `;` (display, clipboard, execution through the protocol).
    #[default]
    Semicolon,
    /// MySQL client file output: a statement whose text holds `;` (a routine, trigger or
    /// event body) is wrapped in `DELIMITER` lines.
    MySqlDelimiter,
}

/// The output of a producer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Script {
    pub stmts: Vec<Stmt>,
    pub notices: Vec<Notice>,
}

impl Script {
    /// The script as text: each statement, its terminator and `\n`, statements separated by
    /// one empty line. Byte-stable: no locale, no clock, no hash order.
    pub fn text(&self, style: DelimStyle) -> String {
        match style {
            DelimStyle::Semicolon => join_semicolon(&self.stmts),
            DelimStyle::MySqlDelimiter => join_mysql(&self.stmts),
        }
    }
}

pub(crate) fn join_semicolon(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    for (i, s) in stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&s.text);
        out.push_str(";\n");
    }
    out
}

/// The first delimiter of a fixed list that the text does not contain.
fn delimiter_for(text: &str) -> &'static str {
    const CANDIDATES: [&str; 5] = [";;", "$$", "//", "@@", "%%"];
    CANDIDATES
        .iter()
        .copied()
        .find(|d| !text.contains(d))
        .unwrap_or("$$$$")
}

fn join_mysql(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    for (i, s) in stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if s.delimiter == Delim::Block {
            let d = delimiter_for(&s.text);
            out.push_str("DELIMITER ");
            out.push_str(d);
            out.push('\n');
            out.push_str(&s.text);
            out.push_str(d);
            out.push_str("\nDELIMITER ;\n");
        } else {
            out.push_str(&s.text);
            out.push_str(";\n");
        }
    }
    out
}
