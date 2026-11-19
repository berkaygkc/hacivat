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
                let end = rest.find("*/").ok_or(K::UnterminatedLiteral)?;
                if rest[..end].contains("/*") {
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
enum Tok<'a> {
    Word(&'a str),
    Num(u32),
    Op(CmpOp),
    Open,
    Close,
}

fn lex_cond(s: &str) -> Option<Vec<Tok<'_>>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' => i += 1,
            b'(' => {
                out.push(Tok::Open);
                i += 1;
            }
            b')' => {
                out.push(Tok::Close);
                i += 1;
            }
            b'0'..=b'9' => {
                let mut e = i;
                while e < b.len() && b[e].is_ascii_digit() {
                    e += 1;
                }
                out.push(Tok::Num(s[i..e].parse().ok()?));
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
                out.push(Tok::Op(op));
                i += if two { 2 } else { 1 };
            }
            _ if is_ident_start(c) => {
                let e = ident_end(b, i);
                out.push(Tok::Word(&s[i..e]));
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
    toks: Vec<Tok<'a>>,
    pos: usize,
}

impl<'a> CondParser<'a> {
    fn peek(&self) -> Option<&Tok<'a>> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok<'a>> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn word(&mut self) -> Result<&'a str, PackErrorKind> {
        match self.bump() {
            Some(Tok::Word(w)) => Ok(w),
            _ => Err(PackErrorKind::BadCondition),
        }
    }

    fn expr(&mut self, depth: usize) -> Result<Cond, PackErrorKind> {
        if depth > MAX_EXPR_DEPTH {
            return Err(PackErrorKind::ExprTooDeep);
        }
        let mut arms = vec![self.and(depth)?];
        while self.peek() == Some(&Tok::Word("or")) {
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
        while self.peek() == Some(&Tok::Word("and")) {
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
            Some(Tok::Word("not")) => {
                self.pos += 1;
                Ok(Cond::Not(Box::new(self.unary(depth + 1)?)))
            }
            Some(Tok::Open) => {
                self.pos += 1;
                let c = self.expr(depth + 1)?;
                match self.bump() {
                    Some(Tok::Close) => Ok(c),
                    _ => Err(PackErrorKind::BadCondition),
                }
            }
            _ => self.atom(),
        }
    }

    fn op(&mut self) -> Result<CmpOp, PackErrorKind> {
        match self.bump() {
            Some(Tok::Op(op)) => Ok(op),
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
                    Some(Tok::Num(n)) => Ok(Cond::Version(op, n)),
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
                if self.bump() != Some(Tok::Open) {
                    return Err(Bad);
                }
                let f = match self.word()? {
                    "mariadb" => PackFlags::MARIADB,
                    "joined" => PackFlags::JOINED,
                    _ => return Err(Bad),
                };
                if self.bump() != Some(Tok::Close) {
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
fn finish(file: &'static str, open: Open) -> Result<Section, PackError> {
    if let Some(f) = open.stack.last() {
        return Err(err(file, f.line, PackErrorKind::UnclosedIf));
    }
    if !open.has_returns {
        return Err(err(file, open.sec.line, PackErrorKind::MissingReturns));
    }
    let sec = open.sec;
    let mut lists: Option<Name> = None;
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
            if let Seg::List(name) = s {
                match &lists {
                    None => lists = Some(name.clone()),
                    Some(n) if n == name => {}
                    Some(_) => {
                        bad = Some((l.line, PackErrorKind::MultipleListBinds));
                        return;
                    }
                }
            }
        }
    });
    if let Some((line, kind)) = bad {
        return Err(err(file, line, kind));
    }
    Ok(sec)
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
                            out.push(finish(file, o)?);
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
            out.push(finish(file, o)?);
        }
    }
    Ok(out)
}
