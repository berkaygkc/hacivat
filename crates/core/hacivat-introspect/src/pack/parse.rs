//! The pack parser (M4 Task 4): line classification, the fail-closed SQL lexer, the condition
//! language and the section tree. Session-free (fuzzed by `fuzz/meta` `pack_parse`).
//!
//! Every byte the lexer treats specially is ASCII, so every slice it takes falls on a UTF-8
//! boundary; non-ASCII text passes through as code or literal content untouched.

use std::collections::HashSet;
use std::sync::Arc;

use hacivat_meta::{Level, Name};
use hacivat_proto::DbmsKind;

use super::{ColDecl, ColType, MAX_NESTING, Mode, PackError, PackErrorKind, PackFlags, Section};

/// The placeholder and comment rules a pack is lexed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Family {
    Pg,
    MySql,
    Sqlite,
}

impl Family {
    pub(crate) fn of(d: sql_dialect::DialectId) -> Option<Family> {
        use sql_dialect::DialectId as D;
        match d {
            D::Postgres => Some(Family::Pg),
            D::MySql | D::MariaDb => Some(Family::MySql),
            D::Sqlite => Some(Family::Sqlite),
            _ => None,
        }
    }

    /// The flavors a server of this family can report (`reachable_variants`).
    pub(crate) fn flavors(self) -> &'static [DbmsKind] {
        match self {
            Family::Pg => &[
                DbmsKind::Postgres,
                DbmsKind::Redshift,
                DbmsKind::Cockroach,
                DbmsKind::Yugabyte,
                DbmsKind::Timescale,
            ],
            Family::MySql => &[DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::TiDb],
            Family::Sqlite => &[DbmsKind::Sqlite],
        }
    }
}

/// The closed set of pack macros (alg/02 HV-INTRO-04: "validated literal substitution from a
/// closed set"; never a user string, except the quoted identifier of R-M4-37).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Macro {
    /// `#{max_in_list}`: the list-bind chunk size, [`super::MAX_IN_LIST`].
    MaxInList,
    /// `#{ident:<bind>}`: the bind's text as a MySQL backtick-quoted identifier; only in
    /// `--- show-create ---` sections (R-M4-37).
    Ident(Name),
}

impl Macro {
    fn parse(body: &str) -> Option<Macro> {
        if body == "max_in_list" {
            return Some(Macro::MaxInList);
        }
        let name = body.strip_prefix("ident:")?;
        is_ident(name).then(|| Macro::Ident(Arc::from(name)))
    }
}

/// One piece of a section line after substitution points are cut out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seg {
    /// Pack text (code, literals, comments) sent as written.
    Text(Arc<str>),
    /// `:name`.
    Bind(Name),
    /// `:[name]`.
    List(Name),
    Macro(Macro),
}

/// A section body node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    Line(LineNode),
    If(IfNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineNode {
    pub(crate) line: u32,
    /// The line as written (the lint re-lexes it).
    pub(crate) raw: Arc<str>,
    pub(crate) segs: Vec<Seg>,
    /// Holds something other than whitespace and comments.
    pub(crate) code: bool,
    /// Holds a literal, quoted identifier, comment or bind (refused in `show-create`), or an
    /// identifier macro glued to other text.
    pub(crate) show_create_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IfNode {
    pub(crate) line: u32,
    pub(crate) arms: Vec<(Cond, Vec<Node>)>,
    pub(crate) else_: Option<Vec<Node>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CmpOp {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
    Ne,
}

impl CmpOp {
    pub(crate) fn test<T: Ord>(self, a: T, b: T) -> bool {
        match self {
            CmpOp::Ge => a >= b,
            CmpOp::Gt => a > b,
            CmpOp::Le => a <= b,
            CmpOp::Lt => a < b,
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
        }
    }
}

/// A `--# if` condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cond {
    Version(CmpOp, u32),
    Level(CmpOp, Level),
    /// `mode == x` (`true`) or `mode != x` (`false`).
    Mode(bool, Mode),
    Flavor(bool, DbmsKind),
    Flag(PackFlags),
    Not(Box<Cond>),
    And(Vec<Cond>),
    Or(Vec<Cond>),
}

/// A lexed piece of one SQL line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Code(&'a str),
    /// A string literal or quoted identifier, quotes included.
    Quoted(&'a str),
    /// A `--` comment (to the end of the line) or a `/* … */` comment.
    Comment(&'a str),
    Bind(&'a str),
    List(&'a str),
    /// The text between `#{` and `}`.
    Macro(&'a str),
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `[A-Za-z_][A-Za-z0-9_]*`.
pub(crate) fn is_ident(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && is_ident_start(b[0]) && b.iter().all(|&c| is_ident_byte(c))
}

fn ident_end(b: &[u8], from: usize) -> usize {
    let mut e = from;
    while e < b.len() && is_ident_byte(b[e]) {
        e += 1;
    }
    e
}

/// Lexes one SQL line of a section (fail-closed: see the module docs of `pack`).
pub(crate) fn scan_line(f: Family, line: &str) -> Result<Vec<Piece<'_>>, PackErrorKind> {
    use PackErrorKind as K;
    let b = line.as_bytes();
    let n = b.len();
    // A bare `\r` ends a PostgreSQL `--` comment, other controls are whitespace to some
    // servers and not to others: only the tab is allowed, so line ends are the lexer's.
    if b.iter().any(|&c| (c < 0x20 && c != b'\t') || c == 0x7f) {
        return Err(K::ControlChar);
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    fn flush<'a>(out: &mut Vec<Piece<'a>>, line: &'a str, start: usize, i: usize) {
        if start < i {
            out.push(Piece::Code(&line[start..i]));
        }
    }
    while i < n {
        let c = b[i];
        let next = b.get(i + 1).copied();
        match c {
            // PostgreSQL has no backtick quote: `` ` `` is an operator character there, so a
            // backtick "quote" would hide code from this lexer (fix round 1, I1a).
            b'`' if f == Family::Pg => return Err(K::Backtick),
            b'\'' | b'"' | b'`' => {
                flush(&mut out, line, start, i);
                let mut j = i + 1;
                loop {
                    let Some(&d) = b.get(j) else {
                        return Err(K::UnterminatedLiteral);
                    };
                    if d == c {
                        if b.get(j + 1) == Some(&c) {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    if d == b'\\' && c != b'`' {
                        return Err(K::BackslashInLiteral);
                    }
                    j += 1;
                }
                let text = &line[i..=j];
                if text.contains("#{") {
                    return Err(K::MacroInLiteral);
                }
                out.push(Piece::Quoted(text));
                i = j + 1;
                start = i;
            }
            b'-' if next == Some(b'-') => {
                if f == Family::MySql && !matches!(b.get(i + 2), None | Some(b' ' | b'\t')) {
                    return Err(K::AmbiguousDashDash);
                }
                flush(&mut out, line, start, i);
                let text = &line[i..];
                if text.contains("#{") {
                    return Err(K::MacroInLiteral);
                }
                out.push(Piece::Comment(text));
                i = n;
                start = n;
            }
            b'/' if next == Some(b'*') => {
                flush(&mut out, line, start, i);
                let rest = &line[i + 2..];
                // MySQL runs the text of `/*!…*/` and MariaDB that of `/*M!…*/` as code
                // (fix round 1, I1d).
                if f == Family::MySql && (rest.starts_with('!') || rest.starts_with("M!")) {
                    return Err(K::ExecutableComment);
                }
                let end = rest.find("*/").ok_or(K::UnterminatedLiteral)?;
                // PostgreSQL nests comments and reads `/*` wherever it starts, also inside the
                // closing `*/` of `/* a /*/`: the check covers the closer too (I1b).
                if rest[..end + 2].contains("/*") {
                    return Err(K::NestedComment);
                }
                let text = &line[i..i + 2 + end + 2];
                if text.contains("#{") {
                    return Err(K::MacroInLiteral);
                }
                out.push(Piece::Comment(text));
                i += end + 4;
                start = i;
            }
            b':' => {
                if next == Some(b':') {
                    i += 2;
                } else if next == Some(b'[') {
                    let e = ident_end(b, i + 2);
                    if e == i + 2 || !is_ident_start(b[i + 2]) || b.get(e) != Some(&b']') {
                        return Err(K::BadBind);
                    }
                    flush(&mut out, line, start, i);
                    out.push(Piece::List(&line[i + 2..e]));
                    i = e + 1;
                    start = i;
                } else if next.is_some_and(|c| c.is_ascii_digit() || c >= 0x80) {
                    // `:1`, `:é` are SQLite parameters (I1c); no dialect needs them as code.
                    return Err(K::RawPlaceholder);
                } else if next.is_some_and(is_ident_start) {
                    let e = ident_end(b, i + 1);
                    flush(&mut out, line, start, i);
                    out.push(Piece::Bind(&line[i + 1..e]));
                    i = e;
                    start = i;
                } else {
                    i += 1;
                }
            }
            b'#' => {
                if next == Some(b'{') {
                    let rest = &line[i + 2..];
                    let end = rest.find('}').ok_or(K::UnknownMacro)?;
                    flush(&mut out, line, start, i);
                    out.push(Piece::Macro(&rest[..end]));
                    i += end + 3;
                    start = i;
                } else if f == Family::MySql {
                    return Err(K::HashComment);
                } else if f == Family::Sqlite {
                    // `#name` is a SQLite parameter (I1c).
                    return Err(K::RawPlaceholder);
                } else {
                    i += 1;
                }
            }
            b'?' if f != Family::Pg => return Err(K::RawPlaceholder),
            b'$' if f != Family::MySql => return Err(K::RawPlaceholder),
            b'@' if f == Family::Sqlite => return Err(K::RawPlaceholder),
            b'[' if f == Family::Sqlite => return Err(K::BracketIdentifier),
            b';' => return Err(K::Semicolon),
            _ => i += 1,
        }
    }
    flush(&mut out, line, start, n);
    Ok(out)
}

/// The words of `text` (ASCII identifier runs; anything else separates), last first.
fn last_words(text: &str) -> impl Iterator<Item = &str> {
    text.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// Converts a lexed line into segments, checking the bind and macro rules.
fn line_node(line_no: u32, raw: &str, pieces: &[Piece<'_>]) -> Result<LineNode, PackErrorKind> {
    use PackErrorKind as K;
    let mut segs: Vec<Seg> = Vec::new();
    let mut text = String::new();
    let mut code = false;
    let mut show_create_text = false;
    // The line's code so far, with literals and comments blanked: what precedes a list bind.
    let mut before = String::new();
    let push_text = |segs: &mut Vec<Seg>, text: &mut String| {
        if !text.is_empty() {
            segs.push(Seg::Text(Arc::from(std::mem::take(text).as_str())));
        }
    };
    for (k, p) in pieces.iter().enumerate() {
        match *p {
            Piece::Code(t) => {
                code |= !t.trim().is_empty();
                text.push_str(t);
                before.push_str(t);
            }
            Piece::Quoted(t) => {
                code = true;
                show_create_text = true;
                text.push_str(t);
                before.push_str(" '' ");
            }
            Piece::Comment(t) => {
                show_create_text = true;
                text.push_str(t);
                before.push(' ');
            }
            Piece::Bind(name) => {
                code = true;
                show_create_text = true;
                push_text(&mut segs, &mut text);
                segs.push(Seg::Bind(Arc::from(name)));
                before.push_str(" :b ");
            }
            Piece::List(name) => {
                code = true;
                show_create_text = true;
                // `<operand> IN :[name]` on one line: with no operand on the line, a `NOT`
                // on the line above could turn it into a chunked `NOT IN`.
                {
                    let mut words = last_words(&before);
                    match (words.next(), words.next()) {
                        (Some(w), Some(op)) if w.eq_ignore_ascii_case("in") => {
                            if op.eq_ignore_ascii_case("not") {
                                return Err(K::NotInListBind);
                            }
                        }
                        _ => return Err(K::ListBindNotAfterIn),
                    }
                }
                push_text(&mut segs, &mut text);
                segs.push(Seg::List(Arc::from(name)));
                before.push_str(" (l) ");
            }
            Piece::Macro(body) => {
                code = true;
                let m = Macro::parse(body).ok_or(K::UnknownMacro)?;
                if matches!(m, Macro::Ident(_)) {
                    let glued_before = match k.checked_sub(1).map(|j| pieces[j]) {
                        None => false,
                        Some(Piece::Code(t)) => {
                            !matches!(t.as_bytes().last(), Some(b' ' | b'\t' | b'.' | b'('))
                        }
                        Some(_) => true,
                    };
                    let glued_after = match pieces.get(k + 1) {
                        None => false,
                        Some(Piece::Code(t)) => !matches!(
                            t.as_bytes().first(),
                            Some(b' ' | b'\t' | b'.' | b')' | b',')
                        ),
                        Some(_) => true,
                    };
                    show_create_text |= glued_before || glued_after;
                }
                push_text(&mut segs, &mut text);
                segs.push(Seg::Macro(m));
                before.push_str(" m ");
            }
        }
    }
    push_text(&mut segs, &mut text);
    Ok(LineNode {
        line: line_no,
        raw: Arc::from(raw),
        segs,
        code,
        show_create_text,
    })
}

// ------------------------------------------------------------------ conditions

#[derive(Debug, Clone, PartialEq, Eq)]
enum CondTok<'a> {
    Word(&'a str),
    Num(u32),
    Op(CmpOp),
    Open,
    Close,
}

fn lex_cond(s: &str) -> Option<Vec<CondTok<'_>>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' => i += 1,
            b'(' => {
                out.push(CondTok::Open);
                i += 1;
            }
            b')' => {
                out.push(CondTok::Close);
                i += 1;
            }
            b'0'..=b'9' => {
                let mut e = i;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                out.push(CondTok::Num(s[i..e].parse().ok()?));
                i = e;
            }
            b'>' | b'<' | b'=' | b'!' => {
                let two = b.get(i + 1) == Some(&b'=');
                let op = match (c, two) {
                    (b'>', true) => CmpOp::Ge,
                    (b'>', false) => CmpOp::Gt,
                    (b'<', true) => CmpOp::Le,
                    (b'<', false) => CmpOp::Lt,
                    (b'=', true) => CmpOp::Eq,
                    (b'!', true) => CmpOp::Ne,
                    _ => return None,
                };
                out.push(CondTok::Op(op));
                i += if two { 2 } else { 1 };
            }
            _ if is_ident_start(c) => {
                let e = ident_end(b, i);
                out.push(CondTok::Word(&s[i..e]));
                i = e;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Parentheses and `not` nest at most this deep in one condition.
const MAX_EXPR_DEPTH: usize = 8;

struct CondParser<'a> {
    toks: Vec<CondTok<'a>>,
    pos: usize,
}

impl<'a> CondParser<'a> {
    fn peek(&self) -> Option<&CondTok<'a>> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<CondTok<'a>> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn word(&mut self) -> Result<&'a str, PackErrorKind> {
        match self.bump() {
            Some(CondTok::Word(w)) => Ok(w),
            _ => Err(PackErrorKind::BadCondition),
        }
    }

    fn expr(&mut self, depth: usize) -> Result<Cond, PackErrorKind> {
        if depth > MAX_EXPR_DEPTH {
            return Err(PackErrorKind::ExprTooDeep);
        }
        let mut arms = vec![self.and(depth)?];
        while self.peek() == Some(&CondTok::Word("or")) {
            self.pos += 1;
            arms.push(self.and(depth)?);
        }
        Ok(if arms.len() == 1 {
            arms.remove(0)
        } else {
            Cond::Or(arms)
        })
    }

    fn and(&mut self, depth: usize) -> Result<Cond, PackErrorKind> {
        let mut arms = vec![self.unary(depth)?];
        while self.peek() == Some(&CondTok::Word("and")) {
            self.pos += 1;
            arms.push(self.unary(depth)?);
        }
        Ok(if arms.len() == 1 {
            arms.remove(0)
        } else {
            Cond::And(arms)
        })
    }

    fn unary(&mut self, depth: usize) -> Result<Cond, PackErrorKind> {
        if depth > MAX_EXPR_DEPTH {
            return Err(PackErrorKind::ExprTooDeep);
        }
        match self.peek() {
            Some(CondTok::Word("not")) => {
                self.pos += 1;
                Ok(Cond::Not(Box::new(self.unary(depth + 1)?)))
            }
            Some(CondTok::Open) => {
                self.pos += 1;
                let c = self.expr(depth + 1)?;
                match self.bump() {
                    Some(CondTok::Close) => Ok(c),
                    _ => Err(PackErrorKind::BadCondition),
                }
            }
            _ => self.atom(),
        }
    }

    fn op(&mut self) -> Result<CmpOp, PackErrorKind> {
        match self.bump() {
            Some(CondTok::Op(op)) => Ok(op),
            _ => Err(PackErrorKind::BadCondition),
        }
    }

    fn eq_op(&mut self) -> Result<bool, PackErrorKind> {
        match self.op()? {
            CmpOp::Eq => Ok(true),
            CmpOp::Ne => Ok(false),
            _ => Err(PackErrorKind::BadCondition),
        }
    }

    fn atom(&mut self) -> Result<Cond, PackErrorKind> {
        use PackErrorKind::BadCondition as Bad;
        match self.word()? {
            "version" => {
                let op = self.op()?;
                match self.bump() {
                    Some(CondTok::Num(n)) => Ok(Cond::Version(op, n)),
                    _ => Err(Bad),
                }
            }
            "level" => {
                let op = self.op()?;
                let l = match self.word()? {
                    "names" => Level::Names,
                    "structure" => Level::Structure,
                    "sources" => Level::Sources,
                    _ => return Err(Bad),
                };
                Ok(Cond::Level(op, l))
            }
            "mode" => {
                let eq = self.eq_op()?;
                let m = match self.word()? {
                    "full" => Mode::Full,
                    "incremental" => Mode::Incremental,
                    "brief" => Mode::Brief,
                    "system" => Mode::System,
                    _ => return Err(Bad),
                };
                Ok(Cond::Mode(eq, m))
            }
            "flavor" => {
                let eq = self.eq_op()?;
                let f = DbmsKind::from_id(self.word()?).ok_or(Bad)?;
                Ok(Cond::Flavor(eq, f))
            }
            "flag" => {
                if self.bump() != Some(CondTok::Open) {
                    return Err(Bad);
                }
                let f = match self.word()? {
                    "mariadb" => PackFlags::MARIADB,
                    "joined" => PackFlags::JOINED,
                    _ => return Err(Bad),
                };
                if self.bump() != Some(CondTok::Close) {
                    return Err(Bad);
                }
                Ok(Cond::Flag(f))
            }
            _ => Err(Bad),
        }
    }
}

fn parse_cond(s: &str) -> Result<Cond, PackErrorKind> {
    let toks = lex_cond(s).ok_or(PackErrorKind::BadCondition)?;
    if toks.is_empty() {
        return Err(PackErrorKind::BadCondition);
    }
    let mut p = CondParser { toks, pos: 0 };
    let c = p.expr(0)?;
    if p.pos != p.toks.len() {
        return Err(PackErrorKind::BadCondition);
    }
    Ok(c)
}

// ------------------------------------------------------------------ sections

struct Frame {
    line: u32,
    arms: Vec<(Cond, Vec<Node>)>,
    else_: Option<Vec<Node>>,
}

impl Frame {
    fn target(&mut self) -> &mut Vec<Node> {
        match &mut self.else_ {
            Some(body) => body,
            None => &mut self.arms.last_mut().expect("an if has an arm").1,
        }
    }
}

/// A section being parsed.
struct Open {
    sec: Section,
    stack: Vec<Frame>,
    body_started: bool,
    has_returns: bool,
}

impl Open {
    fn target(&mut self) -> &mut Vec<Node> {
        match self.stack.last_mut() {
            Some(f) => f.target(),
            None => &mut self.sec.body,
        }
    }
}

fn err(file: &'static str, line: u32, kind: PackErrorKind) -> PackError {
    PackError { file, line, kind }
}

fn parse_returns(s: &str) -> Option<Vec<ColDecl>> {
    let mut cols: Vec<ColDecl> = Vec::new();
    for part in s.split(',') {
        let mut words = part.split_ascii_whitespace();
        let (Some(name), Some(ty), None) = (words.next(), words.next(), words.next()) else {
            return None;
        };
        if !is_ident(name) || cols.iter().any(|c| &*c.name == name) {
            return None;
        }
        cols.push(ColDecl {
            name: Arc::from(name),
            ty: ColType::from_str(ty)?,
        });
    }
    (!cols.is_empty()).then_some(cols)
}

fn is_section_name(s: &str) -> bool {
    is_ident(s) && s.as_bytes()[0].is_ascii_alphabetic()
}

/// Closes `open`: blocks balanced, `returns` declared, the `show-create` and list rules hold.
fn finish(
    file: &'static str,
    open: Open,
    dialect: sql_dialect::DialectId,
    family: Family,
) -> Result<Section, PackError> {
    if let Some(f) = open.stack.last() {
        return Err(err(file, f.line, PackErrorKind::UnclosedIf));
    }
    if !open.has_returns {
        return Err(err(file, open.sec.line, PackErrorKind::MissingReturns));
    }
    let sec = open.sec;
    let mut lists: Option<Name> = None;
    let mut uses = 0usize;
    let mut bad: Option<(u32, PackErrorKind)> = None;
    walk_lines(&sec.body, &mut |l| {
        if bad.is_some() {
            return;
        }
        if sec.show_create && l.show_create_text {
            bad = Some((l.line, PackErrorKind::ShowCreateText));
            return;
        }
        for s in &l.segs {
            match s {
                Seg::Macro(Macro::Ident(_)) if !sec.show_create => {
                    bad = Some((l.line, PackErrorKind::IdentOutsideShowCreate));
                    return;
                }
                Seg::List(name) => {
                    match &lists {
                        None => lists = Some(name.clone()),
                        Some(n) if n == name => {}
                        Some(_) => {
                            bad = Some((l.line, PackErrorKind::MultipleListBinds));
                            return;
                        }
                    }
                    uses += 1;
                    if uses > 1 {
                        bad = Some((l.line, PackErrorKind::ListBindReused));
                        return;
                    }
                }
                _ => {}
            }
        }
    });
    if let Some((line, kind)) = bad {
        return Err(err(file, line, kind));
    }
    check_session_words(&sec, family).map_err(|(line, k)| err(file, line, k))?;
    check_joinable_calls(&sec, family).map_err(|(line, k)| err(file, line, k))?;
    if lists.is_some() {
        check_list_variants(&sec, dialect, family).map_err(|(line, k)| err(file, line, k))?;
    }
    Ok(sec)
}

/// Words a joined `MetaConn` and the session's read check refuse anywhere in the text (T5):
/// a `joinable` section never holds them, not even in a literal or comment; a value that
/// needs one is a bind.
const JOINED_REFUSED: [&str; 5] = ["insert", "update", "delete", "merge", "into"];

/// T5's carry, enforced at parse time: no `PRAGMA` statement in any pack (SQLite reads
/// through the `pragma_*` table-valued functions), and no write word in a `joinable` section.
fn check_session_words(sec: &Section, family: Family) -> Result<(), (u32, PackErrorKind)> {
    let mut bad = None;
    walk_lines(&sec.body, &mut |l| {
        if bad.is_some() {
            return;
        }
        let mut toks = Vec::new();
        line_tokens(family, l, &mut toks);
        if toks
            .iter()
            .any(|(_, t)| is_word(Some(&(0, *t)), &["pragma"]))
        {
            bad = Some((l.line, PackErrorKind::PragmaStatement));
            return;
        }
        if sec.joinable
            && let Some(w) = l
                .raw
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .find(|w| JOINED_REFUSED.iter().any(|r| w.eq_ignore_ascii_case(r)))
        {
            bad = Some((l.line, PackErrorKind::JoinableWriteWord(w.to_string())));
        }
    });
    bad.map_or(Ok(()), Err)
}

// ------------------------------------------------------------------ list-bind placement

/// A token of a section's code, as the list check and the lint read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok<'a> {
    /// An ASCII word (or a run of non-ASCII bytes), or a quoted identifier's text.
    Word(&'a str),
    Op(&'a str),
    /// A string literal, a scalar bind or a macro: an operand the rules do not read.
    Operand,
    /// A `:[name]` list bind.
    List,
}

const OPS: [&str; 10] = [
    "!~~*", "!~*", "~~*", "~*", "<=", ">=", "<>", "!=", "::", "||",
];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Splits one code piece into words and operators.
pub(crate) fn tokenize<'a>(code: &'a str, line: u32, out: &mut Vec<(u32, Tok<'a>)>) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if is_word_byte(c) {
            let mut e = i;
            while e < b.len() && is_word_byte(b[e]) {
                e += 1;
            }
            out.push((line, Tok::Word(&code[i..e])));
            i = e;
        } else if let Some(op) = OPS.iter().find(|op| code[i..].starts_with(**op)) {
            out.push((line, Tok::Op(op)));
            i += op.len();
        } else {
            out.push((line, Tok::Op(&code[i..i + 1])));
            i += 1;
        }
    }
}

/// The tokens of one parsed line (comments dropped; a `"…"` or `` `…` `` quoted identifier
/// is a word, a string literal an operand).
pub(crate) fn line_tokens<'a>(f: Family, l: &'a LineNode, out: &mut Vec<(u32, Tok<'a>)>) {
    // The parser accepted the line, so it lexes again.
    let Ok(pieces) = scan_line(f, &l.raw) else {
        return;
    };
    for p in pieces {
        match p {
            Piece::Code(c) => tokenize(c, l.line, out),
            Piece::Quoted(q) if q.starts_with('"') || q.starts_with('`') => {
                out.push((l.line, Tok::Word(&q[1..q.len() - 1])));
            }
            Piece::Quoted(_) | Piece::Bind(_) | Piece::Macro(_) => out.push((l.line, Tok::Operand)),
            Piece::List(_) => out.push((l.line, Tok::List)),
            Piece::Comment(_) => {}
        }
    }
}

/// A list-bind section is checked in at most this many `--# if` variants.
const LIST_VARIANT_CAP: usize = 4096;

/// Words that are never part of the column of `<column> IN :[name]`.
const NOT_A_COLUMN: [&str; 22] = [
    "not", "and", "or", "xor", "where", "select", "on", "when", "then", "else", "case", "end",
    "exists", "is", "null", "true", "false", "all", "any", "some", "in", "between",
];

/// The clause keywords; the nearest one before a list bind at its level must be `WHERE`.
const CLAUSES: [&str; 19] = [
    "select",
    "from",
    "where",
    "on",
    "join",
    "using",
    "group",
    "having",
    "order",
    "limit",
    "offset",
    "union",
    "intersect",
    "except",
    "returning",
    "set",
    "values",
    "window",
    "with",
];

/// Words that turn a list-bind conjunct into something else when they stand beside it, at
/// expression depth 0 of the `WHERE` clause.
const SIBLING_REFUSED: [&str; 2] = ["or", "xor"];

/// Words that change a result under chunking anywhere in the list's query (query level 0):
/// grouping, paging, distinctness, windows and aggregate markers, set operations.
const QUERY_REFUSED: [&str; 13] = [
    "group",
    "having",
    "limit",
    "offset",
    "fetch",
    "distinct",
    "over",
    "window",
    "filter",
    "within",
    "union",
    "intersect",
    "except",
];

/// Words followed by `(` that are SQL syntax, not a function call.
pub(crate) const SYNTAX: [&str; 56] = [
    "in",
    "exists",
    "any",
    "all",
    "some",
    "array",
    "values",
    "as",
    "cast",
    "coalesce",
    "nullif",
    "greatest",
    "least",
    "row",
    "over",
    "filter",
    "join",
    "from",
    "using",
    "on",
    "and",
    "or",
    "not",
    "where",
    "select",
    "when",
    "then",
    "else",
    "between",
    "is",
    "case",
    "lateral",
    "by",
    "with",
    "recursive",
    "partition",
    "extract",
    "position",
    "substring",
    "trim",
    "overlay",
    "collation",
    "interval",
    "within",
    "table",
    "into",
    "returning",
    "union",
    "intersect",
    "except",
    "like",
    "distinct",
    "end",
    "having",
    "group",
    // ## m4-t16-pg (named exception, review I-4): `OPERATOR(pg_catalog.=)`, the qualified
    // operator syntax joinable sections use so no operator resolves through `search_path`.
    "operator",
];

/// The function called at `toks[j]`, if `j` is a call: a word (or quoted name) followed by
/// `(` that is not SQL syntax and not a type name (after `::` or `AS`).
pub(crate) fn call_name<'a>(toks: &[(u32, Tok<'a>)], j: usize) -> Option<&'a str> {
    let Tok::Word(w) = toks.get(j)?.1 else {
        return None;
    };
    if toks.get(j + 1).map(|t| t.1) != Some(Tok::Op("("))
        || SYNTAX.iter().any(|s| w.eq_ignore_ascii_case(s))
    {
        return None;
    }
    if j >= 1 && (toks[j - 1].1 == Tok::Op("::") || is_word(toks.get(j - 1), &["as"])) {
        return None;
    }
    Some(w)
}

fn starts_with_ci(w: &str, prefix: &str) -> bool {
    w.len() >= prefix.len()
        && w.is_char_boundary(prefix.len())
        && w[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn ends_with_ci(w: &str, suffix: &str) -> bool {
    w.len() >= suffix.len()
        && w.is_char_boundary(w.len() - suffix.len())
        && w[w.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Row-by-row functions every read section may call where a stricter rule applies (a
/// list-bind query, a `joinable` section): they compute one value per row, so chunking
/// cannot change them, and none writes, locks, sleeps or assigns an XID (fix round 2, I5 and
/// m8: an allowlist, not a denylist of aggregates and side effects). Later tasks extend it by
/// named exception.
const ROW_FUNCTIONS: [&str; 52] = [
    // PostgreSQL
    "age",
    "array_to_string",
    "string_to_array",
    "array_length",
    "array_position",
    "cardinality",
    "unnest",
    "pg_encoding_to_char",
    "pg_relation_size",
    "pg_total_relation_size",
    "pg_table_size",
    "pg_indexes_size",
    "pg_column_size",
    "pg_tablespace_location",
    "current_database",
    "current_schema",
    "pg_current_snapshot",
    "pg_snapshot_xmin",
    "pg_snapshot_xmax",
    "pg_current_xact_id_if_assigned",
    "txid_current_if_assigned",
    "encode",
    "md5",
    "sha256",
    "split_part",
    "starts_with",
    // Shared
    "length",
    "char_length",
    "octet_length",
    "replace",
    "concat",
    "concat_ws",
    "abs",
    "round",
    "floor",
    "ceil",
    "ceiling",
    "mod",
    // MySQL / MariaDB
    "if",
    "ifnull",
    "isnull",
    "substring_index",
    "hex",
    "find_in_set",
    "utc_timestamp",
    "unix_timestamp",
    "crc32",
    "sha2",
    // SQLite
    "iif",
    "substr",
    "instr",
    "typeof",
];

/// PostgreSQL catalog deparsers and lookups: row by row, but their text depends on the
/// `search_path`, which a joined connection does not pin (R-M4-46). Allowed in a list-bind
/// query, not in a `joinable` section.
const DEPARSERS: [&str; 7] = [
    "format_type",
    "obj_description",
    "col_description",
    "shobj_description",
    "quote_ident",
    "to_regclass",
    "to_regtype",
];

/// Whether `f` may be called in a list-bind query (at its level) or, with `joinable`, in a
/// `joinable` section. A `joinable` section may also aggregate (its rows are read whole).
fn function_allowed(f: &str, joinable: bool) -> bool {
    let eq = |s: &&str| f.eq_ignore_ascii_case(s);
    if ROW_FUNCTIONS.iter().any(eq) || starts_with_ci(f, "pragma_") {
        return true;
    }
    if joinable {
        ["count", "max", "min", "sum", "bool_and", "bool_or", "every"]
            .iter()
            .any(eq)
    } else {
        DEPARSERS.iter().any(eq)
            || starts_with_ci(f, "pg_get_")
            || starts_with_ci(f, "pg_identify_object")
            || starts_with_ci(f, "pg_describe_object")
            || (starts_with_ci(f, "has_") && ends_with_ci(f, "_privilege"))
            || (starts_with_ci(f, "pg_") && ends_with_ci(f, "_is_visible"))
    }
}

/// A `joinable` section calls only allowlisted functions (fix round 2, m8): no
/// `set_config`, `nextval`/`setval`, advisory locks, large objects, `dblink`, `pg_notify`,
/// XID-assigning calls, MySQL `GET_LOCK`/`RELEASE_LOCK`/`SLEEP`/`BENCHMARK` — nothing outside
/// the list.
fn check_joinable_calls(sec: &Section, family: Family) -> Result<(), (u32, PackErrorKind)> {
    if !sec.joinable {
        return Ok(());
    }
    let mut toks = Vec::new();
    walk_lines(&sec.body, &mut |l| line_tokens(family, l, &mut toks));
    for j in 0..toks.len() {
        if let Some(f) = call_name(&toks, j)
            && !function_allowed(f, true)
        {
            return Err((toks[j].0, PackErrorKind::JoinableFunction(f.to_string())));
        }
    }
    Ok(())
}

fn is_word(t: Option<&(u32, Tok<'_>)>, any: &[&str]) -> bool {
    matches!(t, Some((_, Tok::Word(w))) if any.iter().any(|a| w.eq_ignore_ascii_case(a)))
}

/// Checks the list-bind placement rule (fix rounds 1 and 2, I3 and I5) in every variant of
/// `sec`.
fn check_list_variants(
    sec: &Section,
    dialect: sql_dialect::DialectId,
    family: Family,
) -> Result<(), (u32, PackErrorKind)> {
    let ctxs = super::render::variants(sec, dialect, family, LIST_VARIANT_CAP)
        .ok_or((sec.line, PackErrorKind::ListBindTooManyVariants))?;
    for ctx in ctxs {
        let mut lines = Vec::new();
        super::render::kept_lines(&sec.body, &ctx, &mut lines, &mut Vec::new());
        let mut toks = Vec::new();
        for l in lines {
            line_tokens(family, l, &mut toks);
        }
        check_list_tokens(&toks, family)?;
    }
    Ok(())
}

/// A list bind is legal only when the top-level `AND` conjunct of the `WHERE` clause that
/// holds it is **exactly** `<column-ref> IN :[name]` (`<column-ref>` one or more `.`-joined
/// identifiers), and its query holds nothing whose result chunking changes. In detail:
/// - expression depth counts `(…)`, `[…]` and `CASE … END`, so a list under a `CASE`, an
///   array constructor or a parenthesis (`NOT (…)`, `(…) IS NOT TRUE`, `EXISTS (…)`) is not
///   at depth 0 and is refused;
/// - the conjunct is preceded by `WHERE` or a conjunction `AND` (not the `AND` of
///   `BETWEEN`) and followed by nothing, `AND` or `ORDER`; the nearest clause keyword before
///   it is `WHERE`;
/// - no `OR`/`XOR` (MySQL `||`) stands at expression depth 0 of the query;
/// - at query level 0 (outside subqueries, inside expressions too) there is no grouping,
///   paging, `DISTINCT` (but `IS [NOT] DISTINCT FROM`), window, `FILTER`, `WITHIN GROUP` or
///   set operation, and every function call is on the row-by-row allowlist (so no aggregate
///   of any family, PostgreSQL, MySQL or SQLite).
///
/// Then every chunk selects a disjoint part of the answer, the union of the chunks is the
/// answer, and an empty list selects nothing.
fn check_list_tokens(toks: &[(u32, Tok<'_>)], family: Family) -> Result<(), (u32, PackErrorKind)> {
    use PackErrorKind as K;
    let Some(li) = toks.iter().position(|t| t.1 == Tok::List) else {
        return Ok(());
    };
    let line = toks[li].0;
    let n = toks.len();
    let mut depths = Vec::with_capacity(n);
    let mut levels = Vec::with_capacity(n);
    let (mut depth, mut level) = (0usize, 0usize);
    let mut parens: Vec<bool> = Vec::new();
    for (j, (_, t)) in toks.iter().enumerate() {
        match t {
            Tok::Op("(") => {
                depths.push(depth);
                levels.push(level);
                depth += 1;
                let sub = is_word(toks.get(j + 1), &["select", "with", "values"]);
                parens.push(sub);
                level += usize::from(sub);
            }
            Tok::Op(")") => {
                depth = depth.saturating_sub(1);
                if parens.pop() == Some(true) {
                    level = level.saturating_sub(1);
                }
                depths.push(depth);
                levels.push(level);
            }
            Tok::Op("[") => {
                depths.push(depth);
                levels.push(level);
                depth += 1;
            }
            Tok::Op("]") => {
                depth = depth.saturating_sub(1);
                depths.push(depth);
                levels.push(level);
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("case") => {
                depths.push(depth);
                levels.push(level);
                depth += 1;
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("end") => {
                depth = depth.saturating_sub(1);
                depths.push(depth);
                levels.push(level);
            }
            _ => {
                depths.push(depth);
                levels.push(level);
            }
        }
    }
    let not_conjunct = Err((line, K::ListBindNotConjunct));
    if depths[li] != 0
        || levels[li] != 0
        || !is_word(li.checked_sub(1).and_then(|k| toks.get(k)), &["in"])
    {
        return not_conjunct;
    }
    // The column reference: identifiers joined by `.`, nothing else.
    let Some(mut k) = li.checked_sub(2) else {
        return not_conjunct;
    };
    loop {
        if !matches!(toks[k].1, Tok::Word(_)) || is_word(toks.get(k), &NOT_A_COLUMN) {
            return not_conjunct;
        }
        if k >= 2 && toks[k - 1].1 == Tok::Op(".") {
            k -= 2;
        } else {
            break;
        }
    }
    // Exactly the conjunct: `WHERE`/`AND` before, `AND`/`ORDER`/the end after.
    if !is_word(
        k.checked_sub(1).and_then(|j| toks.get(j)),
        &["where", "and"],
    ) {
        return not_conjunct;
    }
    // An `AND` that closes `BETWEEN x AND` binds the column into the `BETWEEN`.
    let owner = (0..k.saturating_sub(1))
        .rev()
        .filter(|&j| depths[j] == 0)
        .find(|&j| is_word(toks.get(j), &["and", "where", "between"]));
    if is_word(owner.and_then(|j| toks.get(j)), &["between"]) {
        return not_conjunct;
    }
    match toks.get(li + 1) {
        None => {}
        Some(t) if is_word(Some(t), &["and", "order"]) => {}
        Some(_) => return not_conjunct,
    }
    // (`IS [NOT] DISTINCT FROM` holds a `FROM` that opens no clause.)
    let clause = (0..li).rev().find(|&j| {
        depths[j] == 0
            && is_word(toks.get(j), &CLAUSES)
            && !(is_word(toks.get(j), &["from"])
                && is_word(j.checked_sub(1).and_then(|p| toks.get(p)), &["distinct"]))
    });
    if !clause.is_some_and(|j| is_word(toks.get(j), &["where"])) {
        return not_conjunct;
    }
    for (j, (l, t)) in toks.iter().enumerate() {
        if levels[j] != 0 {
            continue;
        }
        let refused: Option<String> = match t {
            Tok::Word(w)
                if depths[j] == 0 && SIBLING_REFUSED.iter().any(|u| w.eq_ignore_ascii_case(u)) =>
            {
                Some((*w).to_string())
            }
            Tok::Op("||") if depths[j] == 0 && family == Family::MySql => Some("||".to_string()),
            Tok::Word(w) if QUERY_REFUSED.iter().any(|u| w.eq_ignore_ascii_case(u)) => {
                let is_distinct_from = w.eq_ignore_ascii_case("distinct")
                    && j >= 1
                    && is_word(toks.get(j - 1), &["is", "not"]);
                (!is_distinct_from).then(|| (*w).to_string())
            }
            Tok::Word(_) => call_name(toks, j)
                .filter(|f| !function_allowed(f, false))
                .map(str::to_string),
            _ => None,
        };
        if let Some(word) = refused {
            return Err((*l, K::ListBindUnchunkable(word)));
        }
    }
    Ok(())
}

/// Visits every line of `nodes`, every branch included, in text order.
pub(crate) fn walk_lines<'a>(nodes: &'a [Node], f: &mut impl FnMut(&'a LineNode)) {
    for n in nodes {
        match n {
            Node::Line(l) => f(l),
            Node::If(i) => {
                for (_, body) in &i.arms {
                    walk_lines(body, f);
                }
                if let Some(body) = &i.else_ {
                    walk_lines(body, f);
                }
            }
        }
    }
}

/// Visits every condition of `nodes`.
pub(crate) fn walk_conds<'a>(nodes: &'a [Node], f: &mut impl FnMut(&'a Cond)) {
    for n in nodes {
        if let Node::If(i) = n {
            for (c, body) in &i.arms {
                f(c);
                walk_conds(body, f);
            }
            if let Some(body) = &i.else_ {
                walk_conds(body, f);
            }
        }
    }
}

/// Parses `files` (sorted by name here) into sections, in file then definition order.
pub(crate) fn parse_files(
    dialect: sql_dialect::DialectId,
    family: Family,
    files: &[(&'static str, &str)],
) -> Result<Vec<Section>, PackError> {
    let mut order: Vec<&(&'static str, &str)> = files.iter().collect();
    order.sort_by(|a, b| a.0.cmp(b.0));
    for w in order.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(err(w[1].0, 0, PackErrorKind::DuplicateFile));
        }
    }
    let mut seen: HashSet<Name> = HashSet::new();
    let mut out = Vec::new();
    for &&(file, text) in &order {
        let mut open: Option<Open> = None;
        for (idx, raw) in text.lines().enumerate() {
            let line = u32::try_from(idx + 1).unwrap_or(u32::MAX);
            let t = raw.trim_matches([' ', '\t']);
            if let Some(rest) = t.strip_prefix("---") {
                let inner = rest
                    .strip_suffix("---")
                    .filter(|_| t.len() >= 6)
                    .ok_or_else(|| err(file, line, PackErrorKind::MalformedDirective))?
                    .trim_matches([' ', '\t']);
                let (key, value) = match inner.split_once(':') {
                    Some((k, v)) => (
                        k.trim_matches([' ', '\t']),
                        Some(v.trim_matches([' ', '\t'])),
                    ),
                    None => (inner, None),
                };
                match (key, value) {
                    ("name", Some(name)) => {
                        if let Some(o) = open.take() {
                            out.push(finish(file, o, dialect, family)?);
                        }
                        if !is_section_name(name) {
                            return Err(err(file, line, PackErrorKind::BadSectionName));
                        }
                        let name: Name = Arc::from(name);
                        if !seen.insert(name.clone()) {
                            return Err(err(file, line, PackErrorKind::DuplicateSection(name)));
                        }
                        open = Some(Open {
                            sec: Section {
                                name,
                                file,
                                line,
                                returns: Vec::new(),
                                joinable: false,
                                show_create: false,
                                body: Vec::new(),
                            },
                            stack: Vec::new(),
                            body_started: false,
                            has_returns: false,
                        });
                    }
                    ("returns", Some(_)) | ("joinable", None) | ("show-create", None) => {
                        let o = match open.as_mut() {
                            Some(o) if !o.body_started => o,
                            _ => return Err(err(file, line, PackErrorKind::DirectiveOutOfPlace)),
                        };
                        let dup = match key {
                            "returns" => {
                                let dup = o.has_returns;
                                o.sec.returns = parse_returns(value.unwrap_or_default())
                                    .ok_or_else(|| err(file, line, PackErrorKind::BadReturns))?;
                                o.has_returns = true;
                                dup
                            }
                            "joinable" => std::mem::replace(&mut o.sec.joinable, true),
                            _ => {
                                if family != Family::MySql {
                                    return Err(err(file, line, PackErrorKind::ShowCreateNotMySql));
                                }
                                std::mem::replace(&mut o.sec.show_create, true)
                            }
                        };
                        if dup {
                            return Err(err(file, line, PackErrorKind::DuplicateFlag));
                        }
                    }
                    _ => return Err(err(file, line, PackErrorKind::UnknownDirective)),
                }
            } else if let Some(rest) = t.strip_prefix("--#") {
                let o = open
                    .as_mut()
                    .ok_or_else(|| err(file, line, PackErrorKind::TextOutsideSection))?;
                o.body_started = true;
                let rest = rest.trim_matches([' ', '\t']);
                let (word, arg) = match rest.split_once([' ', '\t']) {
                    Some((w, a)) => (w, a.trim_matches([' ', '\t'])),
                    None => (rest, ""),
                };
                let cond = |a: &str| parse_cond(a).map_err(|k| err(file, line, k));
                match word {
                    "if" => {
                        let c = cond(arg)?;
                        if o.stack.len() >= MAX_NESTING {
                            return Err(err(file, line, PackErrorKind::NestingTooDeep));
                        }
                        o.stack.push(Frame {
                            line,
                            arms: vec![(c, Vec::new())],
                            else_: None,
                        });
                    }
                    "elif" => {
                        let c = cond(arg)?;
                        match o.stack.last_mut() {
                            Some(f) if f.else_.is_none() => f.arms.push((c, Vec::new())),
                            _ => return Err(err(file, line, PackErrorKind::UnbalancedBlock)),
                        }
                    }
                    "else" | "end" if !arg.is_empty() => {
                        return Err(err(file, line, PackErrorKind::MalformedDirective));
                    }
                    "else" => match o.stack.last_mut() {
                        Some(f) if f.else_.is_none() => f.else_ = Some(Vec::new()),
                        _ => return Err(err(file, line, PackErrorKind::UnbalancedBlock)),
                    },
                    "end" => {
                        let f = o
                            .stack
                            .pop()
                            .ok_or_else(|| err(file, line, PackErrorKind::UnbalancedBlock))?;
                        let node = Node::If(IfNode {
                            line: f.line,
                            arms: f.arms,
                            else_: f.else_,
                        });
                        o.target().push(node);
                    }
                    _ => return Err(err(file, line, PackErrorKind::UnknownDirective)),
                }
            } else if let Some(o) = open.as_mut() {
                let pieces = scan_line(family, raw).map_err(|k| err(file, line, k))?;
                let node = line_node(line, raw, &pieces).map_err(|k| err(file, line, k))?;
                o.body_started |= node.code;
                o.target().push(Node::Line(node));
            } else if !(t.is_empty() || t.starts_with("--")) {
                return Err(err(file, line, PackErrorKind::TextOutsideSection));
            }
        }
        if let Some(o) = open.take() {
            out.push(finish(file, o, dialect, family)?);
        }
    }
    Ok(out)
}
