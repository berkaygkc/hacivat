//! Documents and runs of edits for the incremental-analysis equivalence checks (Task 5, fix
//! round 1, I1): driven by a seed (a proptest `u64`, or T20's `relex_equivalence` fuzz target's
//! input bytes through [`Rng::from_bytes`]), so every consumer generates the same cases.
//!
//! What it reaches that a handful of pieces does not:
//! - **sizes:** small scripts, scripts of 2–16 KiB (lexer checkpoints every 2 KiB, many
//!   anchors, windows that grow), and runs of statements on one line (more than the
//!   splitter's `ANCHOR_SCAN` items without an anchor between them);
//! - **edits that end where the analysis restarts:** at a checkpoint, a line start or a
//!   statement's start, joining the line before it, inserting at it, deleting up to it;
//! - **runs of edits**, each against the text the one before left.
//!
//! The pieces open and close every construct the lexer and splitter know: `$$` bodies, `COPY`
//! data, psql meta-commands and `\if`, mysql `DELIMITER`, executable comments and hints, quotes,
//! comments, byte-order marks, Turkish letters, and the prefixes a lexer decides by looking
//! ahead (`U&'`, `-- `, `$tag$`, `x'`).

#![allow(dead_code)]

use sql_text::{TextEdit, TextRange, TextSize};

/// SplitMix64.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// A generator seeded from fuzz input.
    pub fn from_bytes(bytes: &[u8]) -> Rng {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for b in bytes {
            seed = (seed ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3);
        }
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Lines and pieces of scripts.
pub const PIECES: &[&str] = &[
    "SELECT 1;\n",
    "SELECT a, b FROM t WHERE x = 'y';\n",
    "select 'it''s';\n",
    "INSERT INTO t VALUES (1, 'a;b');\n",
    "UPDATE t SET a = 1\nWHERE b = 2;\n",
    "SELECT 1\nSELECT 2\n",
    "WITH x AS (SELECT 1) SELECT * FROM x;\n",
    "-- a note\n",
    "/* block\n   comment */\n",
    "/* nested /* inner */ still */\n",
    "\n",
    "\n\n",
    "\r\n",
    "  ",
    "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n",
    "CREATE FUNCTION g() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END;\n",
    "DO $body$\nBEGIN\n  PERFORM 1;\nEND\n$body$;\n",
    "$$",
    "$q$ x; $q$",
    "COPY t FROM STDIN;\n1\tdon't\n\\.\n",
    "COPY t FROM STDIN; SELECT 1;\n",
    "\\.\n",
    "\\set x 'a\n",
    "\\echo don't\n",
    "\\if :x\n",
    "\\else\n",
    "\\endif\n",
    "SELECT 1 \\g\n",
    "SELECT 1 \\gset p_\n",
    "\\copy t from stdin\nrow\n\\.\n",
    "DELIMITER //\n",
    "DELIMITER ;\n",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END//\n",
    "CREATE PROCEDURE q() BEGIN IF x THEN SELECT 1; END IF; END;\n",
    "CREATE TRIGGER tr AFTER INSERT ON a BEGIN UPDATE b SET c = 1; END;\n",
    "/*!50000 SELECT x'\\' */;\n",
    "/*!40101 SET @x = 1 */;\n",
    "/*! , \"*/ x; -- \" */\n",
    "/*+ SET_VAR(a=1) */",
    "/*M! SELECT 1 */;\n",
    "SELECT E'a\\'b';\n",
    "SELECT \"q;\" FROM `t;`;\n",
    "# hash comment\n",
    "source f.sql\n",
    ".mode csv\n",
    "SET sql_mode = 'ANSI_QUOTES';\n",
    "SELECT @@sql_mode;\n",
    "BEGIN;\n",
    "END;\n",
    "END\n",
    "SELECT U",
    "&x FROM t;\n",
    "SELECT 1 -",
    "-x;\n",
    "SELECT $",
    "fn x;\n",
    "'",
    "\"",
    "`",
    "/*",
    "*/",
    "--",
    "x'0F'",
    "\u{feff}",
    "İ",
    "ı",
    ";",
    "\\",
];

/// What an edit inserts.
pub const INSERTS: &[&str] = &[
    "",
    "x",
    " ",
    "\n",
    "\r",
    "\r\n",
    "'",
    "\"",
    "`",
    "$",
    "$$",
    ";",
    "-",
    "--",
    "/*",
    "*/",
    "*",
    "\\",
    "\\.",
    "!",
    "#",
    "&",
    "&'",
    "U",
    "e",
    "x'",
    "\u{feff}",
    "İ",
    "ç",
    "SELECT 2;\n",
    "END",
    "BEGIN",
    "IF",
    "THEN",
    "(",
    ")",
    "DELIMITER $$\n",
    "DELIMITER ;\n",
    "\\set y 1\n",
    "\\if 1\n",
    "COPY t FROM STDIN;\n",
    "/*!",
    "/*+",
    "-- c\n",
    "\n\n",
    "CREATE PROCEDURE r() ",
];

/// A script: small (up to 40 pieces), 2–16 KiB, or with a run of statements on one line.
pub fn doc(r: &mut Rng) -> String {
    let mut s = String::new();
    match r.below(4) {
        0 | 1 => {
            for _ in 0..r.below(41) {
                s.push_str(r.pick(PIECES));
            }
        }
        2 => {
            let want = 2048 + r.below(14 * 1024);
            while s.len() < want {
                // Mostly whole statements, so the analysis has anchors to find.
                if r.below(4) == 0 {
                    s.push_str(r.pick(PIECES));
                } else {
                    s.push_str(r.pick(&PIECES[..8]));
                }
            }
        }
        _ => {
            for _ in 0..r.below(8) {
                s.push_str(r.pick(PIECES));
            }
            // More statements on one line than the splitter scans for an anchor.
            for i in 0..(60 + r.below(80)) {
                s.push_str(&format!("SELECT {i}; "));
            }
            s.push('\n');
            for _ in 0..r.below(40) {
                s.push_str(r.pick(&PIECES[..8]));
            }
        }
    }
    s
}

fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).expect("a test text is under 4 GiB"))
}

/// An edit of `text`. Half of them start or end at a mark (`marks`: checkpoints and statement
/// starts the caller knows, plus the text's line starts): the line before it joined, a piece
/// inserted at it, the text before it deleted. The others are anywhere.
pub fn edit(r: &mut Rng, text: &str, marks: &[usize]) -> TextEdit {
    let ins: String = (0..r.below(3)).map(|_| r.pick(INSERTS)).collect();
    let mark = match r.below(4) {
        0 if !marks.is_empty() => Some(marks[r.below(marks.len())]),
        1 => {
            let b = text.as_bytes();
            let from = r.below(text.len() + 1);
            b[from.min(b.len())..]
                .iter()
                .position(|c| *c == b'\n')
                .map(|lf| from + lf + 1)
        }
        _ => None,
    };
    let (a, b) = match mark.map(|m| floor(text, m)) {
        Some(m) => match r.below(4) {
            0 => (floor(text, m.saturating_sub(1)), m),
            1 => (m, m),
            2 => (floor(text, m.saturating_sub(1 + r.below(64))), m),
            _ => (m, floor(text, m + 1 + r.below(8))),
        },
        None => {
            let a = floor(text, r.below(text.len() + 1));
            let del = match r.below(7) {
                0..=3 => r.below(3),
                4 | 5 => r.below(24),
                _ => r.below(2048),
            };
            (a, floor(text, a + del))
        }
    };
    TextEdit {
        delete: TextRange::new(size(a), size(b.max(a))),
        insert: ins,
    }
}

/// The checkpoint invariant `relex` relies on (fix round 1, C1): every checkpoint of `l` (a
/// lexing of `text`) is a token start at a line start, not at a byte-order mark, in order, and
/// lexing the rest of the text from it in `Initial` gives `l`'s tokens from there on.
pub fn checkpoints_hold(
    text: &str,
    l: &sql_lexer::Lexed,
    p: &sql_dialect::LexDialect,
    m: sql_dialect::LexModes,
) -> Result<(), String> {
    let b = text.as_bytes();
    let mut last = None;
    for cp in l.checkpoints() {
        let c = usize::from(*cp);
        if last.is_some_and(|x| x >= c) {
            return Err(format!("checkpoint {c} out of order"));
        }
        last = Some(c);
        if c == 0 || c >= b.len() || b[c - 1] != b'\n' {
            return Err(format!(
                "checkpoint {c} is not a line start inside the text"
            ));
        }
        if b[c..].starts_with(b"\xEF\xBB\xBF") {
            return Err(format!("checkpoint {c} is at a byte-order mark"));
        }
        let Some(from) = l.token_at(*cp).filter(|i| l.range(*i).start() == *cp) else {
            return Err(format!("checkpoint {c} is not a token start"));
        };
        let tail =
            sql_lexer::lex_with_client(&text[c..], p, m, &sql_lexer::LexState::Initial, l.client());
        let same = tail.len() == l.len() - from
            && (0..tail.len()).all(|i| {
                tail.kind(i) == l.kind(from + i)
                    && tail.range(i) + *cp == l.range(from + i)
                    && tail.flags(i) == l.flags(from + i)
            });
        if !same {
            return Err(format!("lexing from checkpoint {c} differs"));
        }
    }
    Ok(())
}

/// How many cases a property runs: `PROPTEST_CASES` when set (nightly: 100,000), else `pr`.
pub fn cases(pr: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(pr)
}
