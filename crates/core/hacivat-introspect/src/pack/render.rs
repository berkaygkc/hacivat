// ## m4-t04-packs (seam: m4-t01c-core)
//! The pack renderer: a section rendered for a [`PackCtx`] becomes queries. A [`Pack`]
//! (`&'static` pack text) renders [`RenderedQuery`]s, and [`RenderedQuery::as_meta_query`]
//! (feature `session`) is the **only** caller of `MetaQuery::new` (R-M4-7; check-deps
//! `MetaQueryOnlyFromPacks`). A [`DraftPack`] (runtime text) renders [`DraftQuery`]s, which
//! have no way to a `MetaQuery` (fix round 1, I2). The two doctests below pin their error
//! codes (fix round 2, m10): stable rustdoc does not check a `compile_fail` code, so each is
//! also written in a comment, as observed by compiling the snippet (it is the only error);
//! the positive controls (a `DraftQuery`'s `sql()`, `Pack::parse` with a literal) are in
//! `tests/hostile_t04_review.rs` R9.
//!
//! ```compile_fail,E0599
//! // E0599 (observed): no method `as_meta_query` on `DraftQuery`.
//! use hacivat_introspect::pack::{DialectId, DraftPack};
//! let p = DraftPack::parse(DialectId::Postgres, "").unwrap();
//! let q = p.render("S", todo!()).unwrap();
//! let _ = q[0].as_meta_query(); // a DraftQuery is not a MetaQuery source
//! ```
//!
//! ```compile_fail,E0597
//! // E0597 (observed): `runtime` does not live long enough (`Pack::parse` needs `'static`).
//! use hacivat_introspect::pack::{DialectId, Pack};
//! let runtime = String::from("--- name: S ---");
//! let _ = Pack::parse(DialectId::Postgres, &runtime); // runtime text never makes a Pack
//! ```
//!
//! The text is the pack's, never a user's; every name and id is a bind (R-M4-3); the one
//! spliced value is the `show-create` identifier macro, quoted by `sql_ident::quote`
//! (R-M4-37). Rendering builds without `session` so the fuzz target exercises it (ADR 0008
//! `m4-t04`); only the `MetaQuery` conversion is gated.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;

use hacivat_meta::{Level, Name};
use hacivat_proto::DbmsKind;
use hacivat_values::{BindValue, LogicalType};
use sql_dialect::DialectId;
use sql_ident::quote::{QuoteCtx, QuoteError, quote};

#[cfg(feature = "session")]
use hacivat_session::meta::MetaQuery;

use super::parse::{Cond, Family, LineNode, Macro, Node, Seg};
use super::{DraftPack, MAX_IN_LIST, Mode, Pack, PackCtx, PackFlags, Params, Section};

/// One statement of a section rendered from a [`DraftPack`]: pack text with its binds. It can
/// be read, never run.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftQuery {
    sql: String,
    binds: Vec<BindValue>,
    joinable: bool,
}

impl DraftQuery {
    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn binds(&self) -> &[BindValue] {
        &self.binds
    }

    /// The section is marked `--- joinable ---` (R-M4-29).
    pub fn joinable(&self) -> bool {
        self.joinable
    }
}

/// One statement of a section rendered from a [`Pack`] (`&'static` pack text). Built only by
/// [`Pack::render`] (its field is private).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedQuery(DraftQuery);

impl RenderedQuery {
    /// The statement as a [`MetaQuery`], the only input a `MetaConn` accepts.
    #[cfg(feature = "session")]
    pub fn as_meta_query(&self) -> MetaQuery<'_> {
        MetaQuery::new(&self.0.sql, &self.0.binds, self.0.joinable)
    }

    pub fn sql(&self) -> &str {
        &self.0.sql
    }

    pub fn binds(&self) -> &[BindValue] {
        &self.0.binds
    }

    /// The section is marked `--- joinable ---` (R-M4-29).
    pub fn joinable(&self) -> bool {
        self.0.joinable
    }
}

/// Why a section does not render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The pack has no section of that name.
    UnknownSection,
    /// A `:name`, `:[name]` or `#{ident:name}` bind the rendered text uses is missing from
    /// `PackCtx::params` (alg/02 HV-INTRO-04 edge 7: an error, never a panic).
    MissingBind(Name),
    /// A joined connection asked for a section not marked `joinable` (R-M4-29).
    NotJoinable,
    /// The context's dialect is not the pack's (MySQL and MariaDB share one pack).
    DialectMismatch,
    /// `PackFlags::MARIADB` disagrees with `flavor == MariaDb`: the parse-time checks assume
    /// they agree, so such a context could reach a branch no check has read (fix round 2, m7).
    FlavorFlagMismatch,
    /// Every SQL line is gated away for this context: no statement exists, which must never
    /// read as "no rows" (alg/02 HV-INTRO-04 edge 8: absence is deliberate, not silent).
    NoVariant,
    /// `#{ident:…}` in a section not marked `show-create` (R-M4-37; the parser refuses it
    /// too).
    IdentOutsideShowCreate,
    /// The `#{ident:…}` bind is SQL NULL or not text-typed: not a name.
    IdentNotText(Name),
    /// The `#{ident:…}` name holds a NUL, which no quoting can carry.
    IdentNul,
    /// The `#{ident:…}` name is empty.
    IdentEmpty,
    /// `sql_ident::quote` refused or did not produce a backtick-quoted identifier.
    IdentUnquotable,
}

/// A section evaluated for one context, before binds: the text with its substitution points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
    segs: Vec<Seg>,
    /// Some line holds code (not only blanks and comments).
    code: bool,
    /// The list bind the kept text uses (at most one use per section, checked at parse).
    list: Option<Name>,
}

/// The template cache key: the section and every context field but the params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TemplateKey {
    section: usize,
    dialect: DialectId,
    version: u32,
    flavor: DbmsKind,
    mode: Mode,
    level: Level,
    flags: PackFlags,
}

/// The template cache holds at most this many entries; it is emptied when full.
const TEMPLATE_CACHE_CAP: usize = 4096;

pub(crate) fn eval(c: &Cond, ctx: &PackCtx) -> bool {
    match c {
        Cond::Version(op, n) => op.test(ctx.version, *n),
        Cond::Level(op, l) => op.test(ctx.level, *l),
        Cond::Mode(eq, m) => (ctx.mode == *m) == *eq,
        Cond::Flavor(eq, f) => (ctx.flavor == *f) == *eq,
        Cond::Flag(f) => ctx.flags.contains(*f),
        Cond::Not(c) => !eval(c, ctx),
        Cond::And(cs) => cs.iter().all(|c| eval(c, ctx)),
        Cond::Or(cs) => cs.iter().any(|c| eval(c, ctx)),
    }
}

/// The lines `nodes` keep for `ctx`, in order; records each `if`'s chosen arm in `path`.
pub(crate) fn kept_lines<'a>(
    nodes: &'a [Node],
    ctx: &PackCtx,
    out: &mut Vec<&'a LineNode>,
    path: &mut Vec<u32>,
) {
    for n in nodes {
        match n {
            Node::Line(l) => out.push(l),
            Node::If(i) => {
                let arm = i.arms.iter().position(|(c, _)| eval(c, ctx));
                let body = match arm {
                    Some(k) => {
                        path.push(u32::try_from(k).unwrap_or(u32::MAX));
                        Some(&i.arms[k].1)
                    }
                    None => {
                        path.push(u32::MAX);
                        i.else_.as_ref()
                    }
                };
                if let Some(body) = body {
                    kept_lines(body, ctx, out, path);
                }
            }
        }
    }
}

fn template_of(sec: &Section, ctx: &PackCtx) -> Template {
    let mut lines = Vec::new();
    kept_lines(&sec.body, ctx, &mut lines, &mut Vec::new());
    let mut t = Template {
        segs: Vec::new(),
        code: false,
        list: None,
    };
    for l in lines {
        if l.raw.trim().is_empty() {
            continue;
        }
        if !t.segs.is_empty() {
            t.segs.push(Seg::Text(Arc::from("\n")));
        }
        t.code |= l.code;
        for s in &l.segs {
            if let Seg::List(name) = s {
                t.list = Some(name.clone());
            }
        }
        t.segs.extend(l.segs.iter().cloned());
    }
    t
}

fn placeholder(f: Family, n: usize, sql: &mut String) {
    match f {
        Family::Pg => {
            sql.push('$');
            // `usize` formatting is locale-free.
            let _ = write!(sql, "{n}");
        }
        Family::MySql | Family::Sqlite => sql.push('?'),
    }
}

fn scalar<'a>(params: &'a Params, name: &Name) -> Result<&'a BindValue, RenderError> {
    params
        .scalars
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .ok_or_else(|| RenderError::MissingBind(name.clone()))
}

/// The `show-create` identifier: the bind's text, backtick-quoted (R-M4-37). MySQL and
/// MariaDB always quote with backticks; `ANSI_QUOTES` is never trusted (T1a review m2). Only
/// a text-typed bind is a name (fix round 1, m5).
fn ident_text(name: &Name, params: &Params) -> Result<String, RenderError> {
    let BindValue::Typed {
        hint: LogicalType::Text { .. },
        text,
    } = scalar(params, name)?
    else {
        return Err(RenderError::IdentNotText(name.clone()));
    };
    if text.is_empty() {
        return Err(RenderError::IdentEmpty);
    }
    // The default context: MySQL/MariaDB identifiers are backticked whatever `ANSI_QUOTES`
    // says (T7's rule), and the check below refuses anything else.
    let q = quote(DialectId::MySql, text, &QuoteCtx::default()).map_err(|e| match e {
        QuoteError::Nul => RenderError::IdentNul,
        _ => RenderError::IdentUnquotable,
    })?;
    let inner = q
        .strip_prefix('`')
        .and_then(|s| s.strip_suffix('`'))
        .ok_or(RenderError::IdentUnquotable)?;
    // Every backtick inside must be doubled: the name can never close the quote.
    if inner.replace("``", "").contains('`') {
        return Err(RenderError::IdentUnquotable);
    }
    Ok(q)
}

fn build(
    family: Family,
    sec: &Section,
    t: &Template,
    params: &Params,
    chunk: &[&BindValue],
) -> Result<DraftQuery, RenderError> {
    let mut sql = String::new();
    let mut binds = Vec::new();
    for s in &t.segs {
        match s {
            Seg::Text(text) => sql.push_str(text),
            Seg::Bind(name) => {
                binds.push(scalar(params, name)?.clone());
                placeholder(family, binds.len(), &mut sql);
            }
            Seg::List(_) => {
                sql.push('(');
                for (k, v) in chunk.iter().enumerate() {
                    if k > 0 {
                        sql.push_str(", ");
                    }
                    binds.push((*v).clone());
                    placeholder(family, binds.len(), &mut sql);
                }
                sql.push(')');
            }
            Seg::Macro(Macro::MaxInList) => {
                let _ = write!(sql, "{MAX_IN_LIST}");
            }
            Seg::Macro(Macro::Ident(name)) => {
                if !sec.show_create {
                    return Err(RenderError::IdentOutsideShowCreate);
                }
                sql.push_str(&ident_text(name, params)?);
            }
        }
    }
    Ok(DraftQuery {
        sql,
        binds,
        joinable: sec.joinable,
    })
}

impl DraftPack {
    fn template(&self, idx: usize, ctx: &PackCtx) -> Arc<Template> {
        let key = TemplateKey {
            section: idx,
            dialect: ctx.dialect,
            version: ctx.version,
            flavor: ctx.flavor,
            mode: ctx.mode,
            level: ctx.level,
            flags: ctx.flags,
        };
        let mut cache = self
            .inner
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = cache.get(&key) {
            return t.clone();
        }
        let t = Arc::new(template_of(&self.inner.sections[idx], ctx));
        if cache.len() >= TEMPLATE_CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, t.clone());
        t
    }

    /// Renders `section` for `ctx`: one query, or several when a list bind is chunked
    /// (R-M4-3: duplicates removed first, then at most [`MAX_IN_LIST`] items each); **none**
    /// when the list bind is empty. That empty answer is right only because the parser
    /// admits a list bind solely as a top-level `AND` conjunct of the `WHERE` clause, never
    /// under a negation (fix round 1, I3): an empty set there selects nothing. Every scalar
    /// bind is checked even then.
    pub fn render(&self, section: &str, ctx: &PackCtx) -> Result<Vec<DraftQuery>, RenderError> {
        let family = self.inner.family;
        if Family::of(ctx.dialect) != Some(family) {
            return Err(RenderError::DialectMismatch);
        }
        if ctx.flags.contains(PackFlags::MARIADB) != (ctx.flavor == DbmsKind::MariaDb) {
            return Err(RenderError::FlavorFlagMismatch);
        }
        let idx = *self
            .inner
            .index
            .get(section)
            .ok_or(RenderError::UnknownSection)?;
        let sec = &self.inner.sections[idx];
        if ctx.flags.contains(PackFlags::JOINED) && !sec.joinable {
            return Err(RenderError::NotJoinable);
        }
        let t = self.template(idx, ctx);
        if !t.code {
            return Err(RenderError::NoVariant);
        }
        // Checks every scalar and macro (and is the answer when there is no list).
        let first = build(family, sec, &t, &ctx.params, &[])?;
        let Some(list) = &t.list else {
            return Ok(vec![first]);
        };
        let items = ctx
            .params
            .lists
            .iter()
            .find(|(n, _)| n == list)
            .map(|(_, v)| v)
            .ok_or_else(|| RenderError::MissingBind(list.clone()))?;
        let mut seen = HashSet::new();
        let unique: Vec<&BindValue> = items.iter().filter(|v| seen.insert(*v)).collect();
        unique
            .chunks(MAX_IN_LIST)
            .map(|chunk| build(family, sec, &t, &ctx.params, chunk))
            .collect()
    }

    /// One context per distinct path through `section`'s `--# if` blocks, so that rendering
    /// each covers every reachable branch (alg/02 HV-INTRO-04 step 4: the CI matrix prepares
    /// each). Params are empty; the caller binds. Version candidates are every threshold the
    /// section names and its neighbours, plus 0; `mode` and `level` take every value when a
    /// condition names them; `flavor` the dialect's own, every named one (`flag(mariadb)`
    /// names MariaDB) and one unnamed member of its family; `flag(mariadb)` follows the
    /// flavor; `flag(joined)` is tried only on a `joinable` section. An unknown section
    /// answers no context.
    pub fn reachable_variants(&self, section: &str) -> Vec<PackCtx> {
        let Some(&idx) = self.inner.index.get(section) else {
            return Vec::new();
        };
        variants(
            &self.inner.sections[idx],
            self.inner.dialect,
            self.inner.family,
            usize::MAX,
        )
        .unwrap_or_default()
    }
}

impl Pack {
    /// Renders `section` for `ctx` (see [`DraftPack::render`]); the queries may run.
    pub fn render(&self, section: &str, ctx: &PackCtx) -> Result<Vec<RenderedQuery>, RenderError> {
        Ok(self
            .draft
            .render(section, ctx)?
            .into_iter()
            .map(RenderedQuery)
            .collect())
    }
}

/// The distinct-path contexts of `sec` (see [`DraftPack::reachable_variants`]); `None` when
/// the candidate product exceeds `cap`.
pub(crate) fn variants(
    sec: &Section,
    dialect: DialectId,
    family: Family,
    cap: usize,
) -> Option<Vec<PackCtx>> {
    let mut versions: Vec<u32> = vec![0];
    let mut modes = false;
    let mut levels = false;
    let mut flavors: Vec<DbmsKind> = Vec::new();
    let mut joined = false;
    super::parse::walk_conds(&sec.body, &mut |c| {
        collect(c, &mut |a| match a {
            Cond::Version(_, n) => {
                versions.extend([n.saturating_sub(1), *n, n.saturating_add(1)]);
            }
            Cond::Mode(..) => modes = true,
            Cond::Level(..) => levels = true,
            Cond::Flavor(_, f) => flavors.push(*f),
            Cond::Flag(f) => {
                joined |= f.contains(PackFlags::JOINED);
                // `flag(mariadb)` follows the flavor, so it names MariaDB.
                if f.contains(PackFlags::MARIADB) {
                    flavors.push(DbmsKind::MariaDb);
                }
            }
            _ => {}
        })
    });
    versions.sort_unstable();
    versions.dedup();
    let modes: &[Mode] = if modes {
        &[Mode::Full, Mode::Incremental, Mode::Brief, Mode::System]
    } else {
        &[Mode::Full]
    };
    let levels: &[Level] = if levels {
        &[Level::Names, Level::Structure, Level::Sources]
    } else {
        &[Level::Structure]
    };
    let mut cands = vec![default_flavor(dialect)];
    if !flavors.is_empty() {
        for f in flavors {
            if !cands.contains(&f) {
                cands.push(f);
            }
        }
        let named = cands.clone();
        if let Some(other) = family.flavors().iter().find(|f| !named.contains(f)) {
            cands.push(*other);
        }
    }
    let joins: &[bool] = if joined && sec.joinable {
        &[false, true]
    } else {
        &[false]
    };
    let product = versions
        .len()
        .saturating_mul(modes.len())
        .saturating_mul(levels.len())
        .saturating_mul(cands.len())
        .saturating_mul(joins.len());
    if product > cap {
        return None;
    }
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let mut out = Vec::new();
    for &version in &versions {
        for &mode in modes {
            for &level in levels {
                for &flavor in &cands {
                    for &j in joins {
                        let mut flags = PackFlags::empty();
                        flags.set(PackFlags::MARIADB, flavor == DbmsKind::MariaDb);
                        flags.set(PackFlags::JOINED, j);
                        let ctx = PackCtx {
                            dialect,
                            version,
                            flavor,
                            mode,
                            level,
                            flags,
                            params: Params::default(),
                        };
                        let mut path = Vec::new();
                        kept_lines(&sec.body, &ctx, &mut Vec::new(), &mut path);
                        if seen.insert(path) {
                            out.push(ctx);
                        }
                    }
                }
            }
        }
    }
    Some(out)
}

/// Visits every atom of `c`.
fn collect<'a>(c: &'a Cond, f: &mut impl FnMut(&'a Cond)) {
    match c {
        Cond::Not(c) => collect(c, f),
        Cond::And(cs) | Cond::Or(cs) => cs.iter().for_each(|c| collect(c, f)),
        atom => f(atom),
    }
}

fn default_flavor(d: DialectId) -> DbmsKind {
    match d {
        DialectId::MySql => DbmsKind::MySql,
        DialectId::MariaDb => DbmsKind::MariaDb,
        DialectId::Sqlite => DbmsKind::Sqlite,
        _ => DbmsKind::Postgres,
    }
}
