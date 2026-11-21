//! A generated script: its statements and the notices shown beside it (Fluent ids, never
//! English text; never a script that silently lacks part of the object). M4 Task 19 body
//! (spike origin `da358e4`).
//!
//! The text form is byte-stable: no locale, no clock, no hash order; `\n` line endings only.

use crate::render::{Delim, Stmt, join_semicolon};

/// A notice attached to a script; the UI maps `id` to Fluent with `args`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The Fluent message id (`ddl-grants-not-loaded`, `ddl-policies-not-loaded`, …).
    pub id: &'static str,
    /// Named arguments, preformatted (P31).
    pub args: Vec<(&'static str, String)>,
}

impl Notice {
    /// A notice without arguments.
    pub fn new(id: &'static str) -> Self {
        Notice {
            id,
            args: Vec::new(),
        }
    }
}

/// How statements end in the text form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DelimStyle {
    /// Every statement ends with `;` (display, clipboard, execution through the protocol).
    #[default]
    Semicolon,
    /// MySQL client file output: a statement whose text holds `;` (a routine, trigger or
    /// event body; the renderer marked it [`Delim::Custom`]) is wrapped in `DELIMITER` lines
    /// with the delimiter the renderer chose, which the text does not contain.
    MySqlDelimiter,
}

/// The statements of a script, in order, and its notices.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Script {
    pub stmts: Vec<Stmt>,
    pub notices: Vec<Notice>,
}

impl Script {
    /// The script as text: each statement, its terminator and `\n`; statements separated by
    /// one empty line.
    pub fn text(&self, style: DelimStyle) -> String {
        match style {
            DelimStyle::Semicolon => join_semicolon(&self.stmts),
            DelimStyle::MySqlDelimiter => join_mysql(&self.stmts),
        }
    }

    /// Whether the script has the notice `id`.
    pub fn has_notice(&self, id: &str) -> bool {
        self.notices.iter().any(|n| n.id == id)
    }
}

/// `DELIMITER d` / text / `d` / `DELIMITER ;` around each statement the renderer gave a custom
/// delimiter; the others as [`join_semicolon`] writes them. The closing delimiter goes on a line
/// of its own when the text's last line could end in a line comment (`--`, `#`).
fn join_mysql(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    for (i, s) in stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match &s.delimiter {
            Delim::Custom(d) => {
                out.push_str("DELIMITER ");
                out.push_str(d);
                out.push('\n');
                out.push_str(&s.text);
                let last = s.text.rsplit('\n').next().unwrap_or("");
                if last.contains("--") || last.contains('#') {
                    out.push('\n');
                }
                out.push_str(d);
                out.push_str("\nDELIMITER ;\n");
            }
            Delim::Semicolon => out.push_str(&join_semicolon(std::slice::from_ref(s))),
        }
    }
    out
}
