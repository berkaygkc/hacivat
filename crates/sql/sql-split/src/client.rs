//! The client-command pre-pass (SQL-SPLIT-01; Task 8): psql `\cmd`, `\;`, `\g` and `COPY …
//! FROM STDIN` data up to `\.`; the mysql client's `DELIMITER`, `\G`, `\g`, `\c`, `\d`,
//! `source`; sqlite3 `.cmd` lines. The tables are `sql_dialect::ClientProfile`.
//!
//! The pre-pass runs inside the splitter's single forward loop (`splitter`), because what a
//! line is depends on the splitter's state: `DELIMITER`, `source` and `.cmd` count only at
//! statement start, and `COPY` data only follows a `COPY … FROM STDIN` that has ended. Quote and
//! comment context comes from the lexer: a `\` that is a token of its own is outside every
//! string and comment, and a client delimiter is searched for only inside code tokens, as raw
//! text (`END$$` is `END` and the delimiter `$$`). After a command (or data, or a delimiter
//! inside a token) the splitter lexes again from where SQL resumes, so a quote in a command's
//! argument or in a data row never reaches the SQL that follows.
//!
//! This module holds the text-level parts: line geometry and the parsing of one command.

use sql_dialect::split_profile::ClientProfile;

/// Where the line holding `at` ends: its line break (`\n`, or the `\r` of `\r\n`), or the
/// text's end.
pub(crate) fn line_end(text: &str, at: usize) -> usize {
    let b = text.as_bytes();
    match memchr_from(b, at, b'\n') {
        Some(lf) if lf > at && b[lf - 1] == b'\r' => lf - 1,
        Some(lf) => lf,
        None => b.len(),
    }
}

/// The start of the line after the one holding `at` (the text's end on the last line).
pub(crate) fn next_line_start(text: &str, at: usize) -> usize {
    let b = text.as_bytes();
    memchr_from(b, at, b'\n').map_or(b.len(), |lf| lf + 1)
}

/// Whether only spaces and tabs stand between `at` and the start of its line.
pub(crate) fn only_blanks_before(text: &str, at: usize) -> bool {
    text.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|c| **c != b'\n')
        .all(|c| *c == b' ' || *c == b'\t')
}

/// Whether `at` is at the left margin (the text's start, or right after a line feed).
pub(crate) fn at_left_margin(text: &str, at: usize) -> bool {
    at == 0 || text.as_bytes()[at - 1] == b'\n'
}

fn memchr_from(b: &[u8], at: usize, needle: u8) -> Option<usize> {
    b.get(at..)?
        .iter()
        .position(|c| *c == needle)
        .map(|i| at + i)
}

fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// What a backslash command at `at` is, by the client's tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backslash {
    /// psql `\;`.
    InlineSemicolon { end: usize },
    /// Ends the statement; `supported` when Hacivat runs the statement as the client would.
    Terminator { end: usize, supported: bool },
    /// Discards the statement typed so far.
    Clear { end: usize },
    /// mysql `\d x`: the delimiter argument (empty when absent) and the command's end.
    Delimiter { arg: (usize, usize), end: usize },
    /// Any other command, to `end`.
    Command { end: usize },
}

/// Parses the backslash command whose `\` is at `at`.
pub(crate) fn backslash(text: &str, at: usize, cp: &ClientProfile) -> Backslash {
    let b = text.as_bytes();
    let next = b.get(at + 1).copied();
    if cp.inline_semicolon && next == Some(b';') {
        return Backslash::InlineSemicolon { end: at + 2 };
    }
    let eol = line_end(text, at);
    if cp.backslash_line {
        psql(text, at, eol, cp)
    } else {
        one_letter(text, at, eol, cp)
    }
}

/// psql: `\name args` to the end of the line, or to a `\\` outside single quotes (the rest of
/// the line is SQL again); `\\` alone is that separator.
fn psql(text: &str, at: usize, eol: usize, cp: &ClientProfile) -> Backslash {
    let b = text.as_bytes();
    let name_end = (at + 1..eol)
        .find(|i| b[*i].is_ascii_whitespace() || b[*i] == b'\\')
        .unwrap_or(eol);
    if name_end == at + 1 {
        // `\\` (the separator), or a lone `\`.
        let end = if b.get(at + 1) == Some(&b'\\') {
            at + 2
        } else {
            at + 1
        };
        return Backslash::Command { end };
    }
    let mut end = eol;
    let mut quoted = false;
    let mut i = name_end;
    while i < eol {
        match b[i] {
            b'\'' => quoted = !quoted,
            b'\\' if !quoted && b.get(i + 1) == Some(&b'\\') => {
                end = i + 2;
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let name = &text[at + 1..name_end];
    let args = text[name_end..end].trim_start_matches([' ', '\t']);
    let args_empty = args.is_empty() || args == "\\\\";
    if let Some((_, ok)) = cp.backslash_terminators.iter().find(|(n, _)| *n == name) {
        return Backslash::Terminator {
            end,
            supported: *ok && args_empty,
        };
    }
    if cp.backslash_clear.contains(&name) {
        return Backslash::Clear { end };
    }
    if name == "copy" && from_stdin(args) {
        // `\copy … from stdin` reads its rows from the script itself, up to `\.`: they are
        // the command's, never SQL.
        return Backslash::Command {
            end: copy_data_end(text, next_line_start(text, eol)),
        };
    }
    Backslash::Command { end }
}

/// Whether `\copy` arguments read `from stdin` (the script; `pstdin` is psql's own input).
fn from_stdin(args: &str) -> bool {
    let words: Vec<&str> = args
        .split(|c: char| c.is_ascii_whitespace() || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .any(|w| w[0].eq_ignore_ascii_case("from") && w[1].eq_ignore_ascii_case("stdin"))
}

/// The end of psql `COPY` data whose lines start at `from`: after the `\.` that ends it (a line
/// that is `\.` alone), or the text's end.
pub(crate) fn copy_data_end(text: &str, from: usize) -> usize {
    let b = text.as_bytes();
    let mut line = from;
    while line < b.len() {
        if b[line..].starts_with(b"\\.")
            && b.get(line + 2).is_none_or(|c| *c == b'\n' || *c == b'\r')
        {
            return line + 2;
        }
        line = next_line_start(text, line);
    }
    b.len()
}

/// The mysql client: `\` and one letter; the terminators and `\c` take nothing more, `\d` takes
/// the delimiter, the others take the rest of the line.
fn one_letter(text: &str, at: usize, eol: usize, cp: &ClientProfile) -> Backslash {
    let b = text.as_bytes();
    let Some(c) = b.get(at + 1).copied().filter(|c| !c.is_ascii_whitespace()) else {
        return Backslash::Command { end: at + 1 };
    };
    if !c.is_ascii() {
        return Backslash::Command { end: eol };
    }
    let name = &text[at + 1..at + 2];
    if let Some((_, ok)) = cp.backslash_terminators.iter().find(|(n, _)| *n == name) {
        return Backslash::Terminator {
            end: at + 2,
            supported: *ok,
        };
    }
    if cp.backslash_clear.contains(&name) {
        return Backslash::Clear { end: at + 2 };
    }
    if cp.backslash_delimiter.contains(&name) {
        return Backslash::Delimiter {
            arg: delimiter_arg(text, at + 2),
            end: eol,
        };
    }
    let end = if cp.backslash_to_eol { eol } else { at + 2 };
    Backslash::Command { end }
}

/// The delimiter argument after a `DELIMITER` word ending at `from`: after blanks, the next
/// whitespace-free run (empty when the line ends first).
pub(crate) fn delimiter_arg(text: &str, from: usize) -> (usize, usize) {
    let b = text.as_bytes();
    let mut start = from;
    while start < b.len() && is_blank(b[start]) {
        start += 1;
    }
    let mut end = start;
    while end < b.len() && !b[end].is_ascii_whitespace() {
        end += 1;
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sql_dialect::ClientFlavor;
    use sql_dialect::split_profile::client_profile;

    #[test]
    fn line_geometry() {
        let t = "ab\r\ncd\nef";
        assert_eq!(line_end(t, 0), 2);
        assert_eq!(line_end(t, 4), 6);
        assert_eq!(line_end(t, 7), 9);
        assert_eq!(next_line_start(t, 0), 4);
        assert_eq!(next_line_start(t, 7), 9);
        assert!(only_blanks_before(" \tx", 2));
        assert!(!only_blanks_before("a x", 2));
        assert!(only_blanks_before("a\n  x", 4));
        assert!(at_left_margin("a\nb", 2));
        assert!(!at_left_margin("a\n b", 3));
    }

    #[test]
    fn psql_commands() {
        let cp = client_profile(ClientFlavor::Psql);
        assert_eq!(
            backslash("\\echo hi\nSELECT 1", 0, cp),
            Backslash::Command { end: 8 }
        );
        assert_eq!(
            backslash("SELECT 1 \\g\n", 9, cp),
            Backslash::Terminator {
                end: 11,
                supported: true
            }
        );
        assert_eq!(
            backslash("SELECT 1 \\g out.txt\n", 9, cp),
            Backslash::Terminator {
                end: 19,
                supported: false
            }
        );
        assert_eq!(
            backslash("x \\gset p_\n", 2, cp),
            Backslash::Terminator {
                end: 10,
                supported: false
            }
        );
        assert_eq!(
            backslash("1 \\; 2", 2, cp),
            Backslash::InlineSemicolon { end: 4 }
        );
        assert_eq!(backslash("\\r\n", 0, cp), Backslash::Clear { end: 2 });
        // `\\` ends the command; a quoted `\\` does not.
        assert_eq!(
            backslash("\\x \\\\ SELECT 1", 0, cp),
            Backslash::Command { end: 5 }
        );
        assert_eq!(
            backslash("\\echo '\\\\' x", 0, cp),
            Backslash::Command { end: 12 }
        );
        // `\copy … from stdin` takes its data rows, up to `\.`.
        let t = "\\copy t (a) FROM STDIN\nDELETE FROM t\n\\.\nSELECT 1;";
        assert_eq!(backslash(t, 0, cp), Backslash::Command { end: 39 });
        assert_eq!(&t[..39], "\\copy t (a) FROM STDIN\nDELETE FROM t\n\\.");
        let t = "\\copy t from stdin\nrow";
        assert_eq!(backslash(t, 0, cp), Backslash::Command { end: t.len() });
        // `pstdin` (psql's own input) and `to stdout` read nothing from the script.
        assert_eq!(
            backslash("\\copy t from pstdin\nx", 0, cp),
            Backslash::Command { end: 19 }
        );
        assert_eq!(
            backslash("\\copy t to stdout\nx", 0, cp),
            Backslash::Command { end: 17 }
        );
    }

    #[test]
    fn mysql_commands() {
        let cp = client_profile(ClientFlavor::MySqlCli);
        assert_eq!(
            backslash("SELECT 1\\G", 8, cp),
            Backslash::Terminator {
                end: 10,
                supported: true
            }
        );
        assert_eq!(backslash("x \\c y", 2, cp), Backslash::Clear { end: 4 });
        assert_eq!(
            backslash("\\d //\nx", 0, cp),
            Backslash::Delimiter {
                arg: (3, 5),
                end: 5
            }
        );
        assert_eq!(
            backslash("\\. f.sql\nx", 0, cp),
            Backslash::Command { end: 8 }
        );
        assert_eq!(delimiter_arg("DELIMITER  $$; x", 9), (11, 14));
        assert_eq!(delimiter_arg("DELIMITER\n", 9), (9, 9));
    }
}
