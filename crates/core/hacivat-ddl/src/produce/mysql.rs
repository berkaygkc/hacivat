//! MySQL / MariaDB server-text head edits: `skip_definer` and `or_replace` (alg/03 HV-DDL-11).
//!
//! `SHOW CREATE VIEW|FUNCTION|PROCEDURE|TRIGGER|EVENT` prints a complete statement whose head
//! is `CREATE [OR REPLACE] [ALGORITHM = …] [DEFINER = …] [SQL SECURITY …] <object word>`.
//! The only edits Hacivat makes to server text are in that head, and only on request: drop
//! the `DEFINER = user` clause, insert `OR REPLACE`. The head is scanned token by token; if
//! it is not exactly that shape (a comment, an executable comment, an unexpected word), the
//! text is left unchanged and the caller emits `ddl-head-not-edited`. The body after the
//! object word is never looked at.

/// A token of the head: its byte range and kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Word,
    Quoted,
    Punct(u8),
}

struct Scanner<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn skip_ws(&mut self) {
        let b = self.text.as_bytes();
        while self.pos < b.len() && b[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// The next token `(start, end, kind)`; `None` at the end or on anything the head may not
    /// contain (a comment start, a non-ASCII byte outside quotes).
    fn next(&mut self) -> Option<(usize, usize, Tok)> {
        self.skip_ws();
        let b = self.text.as_bytes();
        let start = self.pos;
        let c = *b.get(start)?;
        if c == b'/' || c == b'#' || (c == b'-' && b.get(start + 1) == Some(&b'-')) {
            return None;
        }
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
            let mut end = start;
            while end < b.len()
                && (b[end].is_ascii_alphanumeric() || b[end] == b'_' || b[end] == b'$')
            {
                end += 1;
            }
            self.pos = end;
            return Some((start, end, Tok::Word));
        }
        if c == b'`' || c == b'\'' || c == b'"' {
            let mut end = start + 1;
            loop {
                let x = *b.get(end)?;
                end += 1;
                if x == c {
                    if b.get(end) == Some(&c) {
                        end += 1;
                        continue;
                    }
                    break;
                }
            }
            self.pos = end;
            return Some((start, end, Tok::Quoted));
        }
        if matches!(c, b'=' | b'@' | b'(' | b')') {
            self.pos = start + 1;
            return Some((start, start + 1, Tok::Punct(c)));
        }
        None
    }

    fn word(&self, t: (usize, usize, Tok)) -> &'a str {
        &self.text[t.0..t.1]
    }
}

fn is_word(s: &Scanner<'_>, t: Option<(usize, usize, Tok)>, w: &str) -> bool {
    matches!(t, Some(tok @ (_, _, Tok::Word)) if s.word(tok).eq_ignore_ascii_case(w))
}

const OBJECT_WORDS: [&str; 6] = [
    "VIEW",
    "FUNCTION",
    "PROCEDURE",
    "TRIGGER",
    "EVENT",
    "AGGREGATE",
];

/// The parsed head: where `OR REPLACE` would go, whether it is there, and the definer clause.
struct Head {
    /// Byte offset just after `CREATE`.
    after_create: usize,
    or_replace: bool,
    /// `DEFINER = user` from `DEFINER` to the start of the next token.
    definer: Option<(usize, usize)>,
}

fn parse_head(text: &str) -> Option<Head> {
    let mut s = Scanner { text, pos: 0 };
    let create = s.next();
    if !is_word(&s, create, "CREATE") {
        return None;
    }
    let after_create = create?.1;
    let mut or_replace = false;
    let mut definer = None;
    let mut tok = s.next();
    if is_word(&s, tok, "OR") {
        let t = s.next();
        if !is_word(&s, t, "REPLACE") {
            return None;
        }
        or_replace = true;
        tok = s.next();
    }
    loop {
        let t = tok?;
        if is_word(&s, tok, "ALGORITHM") {
            if s.next()?.2 != Tok::Punct(b'=') || s.next()?.2 != Tok::Word {
                return None;
            }
        } else if is_word(&s, tok, "DEFINER") {
            if s.next()?.2 != Tok::Punct(b'=') {
                return None;
            }
            let user = s.next()?;
            match user.2 {
                Tok::Word if s.word(user).eq_ignore_ascii_case("CURRENT_USER") => {
                    // Optional `()`.
                    let save = s.pos;
                    match s.next() {
                        Some((_, _, Tok::Punct(b'('))) => {
                            if s.next()?.2 != Tok::Punct(b')') {
                                return None;
                            }
                        }
                        _ => s.pos = save,
                    }
                }
                Tok::Word | Tok::Quoted => {
                    let save = s.pos;
                    match s.next() {
                        Some((_, _, Tok::Punct(b'@'))) => {
                            if !matches!(s.next()?.2, Tok::Word | Tok::Quoted) {
                                return None;
                            }
                        }
                        _ => s.pos = save,
                    }
                }
                Tok::Punct(_) => return None,
            }
            s.skip_ws();
            definer = Some((t.0, s.pos));
        } else if is_word(&s, tok, "SQL") {
            let t = s.next();
            if !is_word(&s, t, "SECURITY") || s.next()?.2 != Tok::Word {
                return None;
            }
        } else if OBJECT_WORDS.iter().any(|w| is_word(&s, tok, w)) {
            return Some(Head {
                after_create,
                or_replace,
                definer,
            });
        } else {
            return None;
        }
        tok = s.next();
    }
}

/// The head edit could not be made; the text is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadNotEdited;

/// `text` with the requested head edits. Without a request the text is returned as is (and
/// never parsed).
pub fn edit_head(
    text: &str,
    strip_definer: bool,
    or_replace: bool,
) -> Result<String, HeadNotEdited> {
    if !strip_definer && !or_replace {
        return Ok(text.to_string());
    }
    let head = parse_head(text).ok_or(HeadNotEdited)?;
    let mut out = String::with_capacity(text.len() + 11);
    let insert_or_replace = or_replace && !head.or_replace;
    match (strip_definer, head.definer) {
        (true, Some((a, b))) => {
            out.push_str(&text[..head.after_create]);
            if insert_or_replace {
                out.push_str(" OR REPLACE");
            }
            out.push_str(&text[head.after_create..a]);
            out.push_str(&text[b..]);
        }
        _ => {
            out.push_str(&text[..head.after_create]);
            if insert_or_replace {
                out.push_str(" OR REPLACE");
            }
            out.push_str(&text[head.after_create..]);
        }
    }
    Ok(out)
}

/// Whether server text holds a MySQL / MariaDB executable comment (`/*!…*/`, MariaDB
/// `/*M!…*/`), which the server executes (verification/01 `#mysql-comments`).
pub fn has_executable_comment(text: &str) -> bool {
    text.contains("/*!") || text.contains("/*M!")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: &str = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`";

    #[test]
    fn strips_the_definer_and_nothing_else() {
        assert_eq!(
            edit_head(VIEW, true, false).unwrap(),
            "CREATE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`"
        );
    }

    #[test]
    fn inserts_or_replace_once() {
        assert_eq!(
            edit_head(VIEW, false, true).unwrap(),
            "CREATE OR REPLACE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`"
        );
        let already = "CREATE OR REPLACE VIEW v AS select 1";
        assert_eq!(edit_head(already, false, true).unwrap(), already);
    }

    #[test]
    fn quoted_definers_and_current_user() {
        let p = "CREATE DEFINER='app'@'10.0.%' PROCEDURE `p`() BEGIN SELECT 1; END";
        assert_eq!(
            edit_head(p, true, false).unwrap(),
            "CREATE PROCEDURE `p`() BEGIN SELECT 1; END"
        );
        let c =
            "CREATE DEFINER=CURRENT_USER() TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1";
        assert_eq!(
            edit_head(c, true, false).unwrap(),
            "CREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1"
        );
        let odd = "CREATE DEFINER=`we``ird`@`h` FUNCTION f() RETURNS int RETURN 1";
        assert_eq!(
            edit_head(odd, true, false).unwrap(),
            "CREATE FUNCTION f() RETURNS int RETURN 1"
        );
    }

    #[test]
    fn unexpected_heads_are_left_alone() {
        for t in [
            "CREATE /*!50017 DEFINER=`root`@`%`*/ TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1",
            "CREATE TABLE t (a int)",
            "create definer = root@localhost  something",
            "SELECT 1",
        ] {
            assert_eq!(edit_head(t, true, true), Err(HeadNotEdited), "{t}");
        }
        // The body is never parsed: a DEFINER word after the object word stays.
        let body = "CREATE VIEW v AS select 'DEFINER=`x`@`y`' AS a";
        assert_eq!(edit_head(body, true, false).unwrap(), body);
    }

    #[test]
    fn executable_comments_are_detected() {
        assert!(has_executable_comment("select /*!80000 1 */"));
        assert!(has_executable_comment("select /*M!100000 1 */"));
        assert!(!has_executable_comment("select /* 1 */"));
    }
}
