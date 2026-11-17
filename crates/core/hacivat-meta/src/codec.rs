//! The versioned binary encoding of one namespace: the payload of a catalog-cache row
//! (HV-MODEL-03).
//!
//! ```text
//! envelope  = "HVNS" major:u16le minor:u16le compression:u8 checksum:u64le body
//! body      = strings ns_header:record count:varint obj:record*
//! strings   = count:varint (len:varint utf8)*          every string of the namespace, once
//! record    = len:varint bytes                          fields in order; trailing bytes ignored
//! tagged    = 0 | tag:varint record                     a union value (props, ids, extensions)
//! ```
//!
//! - **Versioning** (alg/02 HV-MODEL-03 "Versioning"): a *major* bump means a new cache file
//!   name (`<profile>.v<major>.sqlite`), so a different major is rejected here. A *minor* bump
//!   is additive: new fields are appended at the end of a record and new union tags are
//!   added. An older reader skips trailing fields (every record is length-prefixed) and maps
//!   an unknown union tag to its default (`Props::None`, `ObjectKind::Unknown`,
//!   `Watermark::None`, which forces a full introspection). A namespace decoded from a newer
//!   minor is marked [`NamespaceSnapshot::from_newer_minor`] and is never written back.
//!   A newer reader reads an older minor's missing trailing fields as their defaults and runs
//!   [`migrate_minor`].
//! - **Strings** are interned per namespace: the 160k columns of a large schema share a few
//!   dozen type texts and a few thousand column names, and the decoder makes one `Arc<str>`
//!   per distinct string.
//! - **Integrity**: XXH3-64 over the body, so a flipped bit drops that row only (HV-MODEL-03
//!   edge 12) instead of decoding garbage. Encryption (sensitive profiles) wraps the whole
//!   envelope, outside this module.
//! - **Compression**: the `compression` byte is `0` (none) in format 1.0. The spec names zstd;
//!   it is not a workspace dependency yet, so it is a manifest decision for M4. A reader
//!   rejects an unknown compression id.
//! - The arena is stored in its breadth-first order with parent indexes; children ranges and
//!   name indexes are rebuilt on load (and the layout is validated).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use smallvec::SmallVec;

use crate::ids::{NsKey, NsKind, Ref, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{FoldPolicy, FoldRule, Name};
use crate::namespace::{
    Level, ModelError, NO_PARENT, NamespaceSnapshot, NsMeta, Obj, ObjFlags, Watermark,
};
use crate::props::*;

/// The cache format major version (a bump means a new cache file).
pub const FORMAT_MAJOR: u16 = 1;
/// The cache format minor version (additive changes).
pub const FORMAT_MINOR: u16 = 0;

const MAGIC: &[u8; 4] = b"HVNS";
const HEADER_LEN: usize = 4 + 2 + 2 + 1 + 8;
const COMPRESSION_NONE: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    /// A different format major (never expected: majors live in different files).
    Major(u16),
    UnknownCompression(u8),
    Checksum,
    Truncated,
    /// Bytes after the objects in a payload of this build's minor or older.
    TrailingBytes,
    VarintOverflow,
    BadUtf8,
    BadStringIndex,
    UnknownDbms,
    BadNsHeader,
    Model(ModelError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::BadMagic => write!(f, "not a namespace payload"),
            DecodeError::Major(m) => write!(f, "format major {m}, expected {FORMAT_MAJOR}"),
            DecodeError::UnknownCompression(c) => write!(f, "unknown compression {c}"),
            DecodeError::Checksum => write!(f, "checksum mismatch"),
            DecodeError::Truncated => write!(f, "truncated payload"),
            DecodeError::TrailingBytes => write!(f, "trailing bytes after the objects"),
            DecodeError::VarintOverflow => write!(f, "integer out of range"),
            DecodeError::BadUtf8 => write!(f, "string is not UTF-8"),
            DecodeError::BadStringIndex => write!(f, "string index out of range"),
            DecodeError::UnknownDbms => write!(f, "unknown DBMS id"),
            DecodeError::BadNsHeader => write!(f, "bad namespace header"),
            DecodeError::Model(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

type DResult<T> = Result<T, DecodeError>;

// ---------------------------------------------------------------------------------------------
// Encoding.

fn put_var(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

struct Enc<'a> {
    index: HashMap<&'a str, u32>,
    list: Vec<&'a str>,
}

impl<'a> Enc<'a> {
    fn intern(&mut self, s: &'a str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(s);
        self.index.insert(s, i);
        i
    }

    fn str(&mut self, out: &mut Vec<u8>, s: &'a str) {
        let i = self.intern(s);
        put_var(out, u64::from(i));
    }

    fn opt_str(&mut self, out: &mut Vec<u8>, s: Option<&'a Name>) {
        match s {
            None => out.push(0),
            Some(s) => {
                let i = self.intern(s);
                put_var(out, u64::from(i) + 1);
            }
        }
    }

    fn strs(&mut self, out: &mut Vec<u8>, v: &'a [Name]) {
        put_var(out, v.len() as u64);
        for s in v {
            self.str(out, s);
        }
    }

    /// A length-prefixed record whose fields `f` writes.
    fn record(&mut self, out: &mut Vec<u8>, f: impl FnOnce(&mut Self, &mut Vec<u8>)) {
        let mut buf = Vec::new();
        f(self, &mut buf);
        put_var(out, buf.len() as u64);
        out.extend_from_slice(&buf);
    }

    /// A tagged union value: `tag` then its record.
    fn tagged(&mut self, out: &mut Vec<u8>, tag: u64, f: impl FnOnce(&mut Self, &mut Vec<u8>)) {
        debug_assert!(tag != 0);
        put_var(out, tag);
        self.record(out, f);
    }
}

fn put_bool(out: &mut Vec<u8>, b: bool) {
    out.push(u8::from(b));
}

fn put_i64(out: &mut Vec<u8>, v: i64) {
    put_var(out, zigzag(v));
}

fn put_opt_i64(out: &mut Vec<u8>, v: Option<i64>) {
    match v {
        None => out.push(0),
        Some(v) => {
            out.push(1);
            put_i64(out, v);
        }
    }
}

fn put_opt_u64(out: &mut Vec<u8>, v: Option<u64>) {
    match v {
        None => out.push(0),
        Some(v) => {
            out.push(1);
            put_var(out, v);
        }
    }
}

fn put_opt_bool(out: &mut Vec<u8>, v: Option<bool>) {
    out.push(match v {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    });
}

fn put_source(out: &mut Vec<u8>, s: Option<SourceRef>) {
    match s {
        None => out.push(0),
        Some(s) => {
            out.push(1);
            out.extend_from_slice(&s.hash.to_le_bytes());
            put_var(out, u64::from(s.len));
        }
    }
}

fn put_server_id(e: &mut Enc<'_>, out: &mut Vec<u8>, id: Option<ServerId>) {
    let Some(id) = id else {
        out.push(0);
        return;
    };
    match id {
        ServerId::Oid(v) => e.tagged(out, 1, |_, o| put_var(o, u64::from(v))),
        ServerId::ObjectId(v) => e.tagged(out, 2, |_, o| put_i64(o, i64::from(v))),
        ServerId::ObjNum(v) => e.tagged(out, 3, |_, o| put_var(o, v)),
        ServerId::Uuid(v) => e.tagged(out, 4, |_, o| o.extend_from_slice(&v)),
        ServerId::RowId(v) => e.tagged(out, 5, |_, o| put_i64(o, v)),
    }
}

fn put_state(e: &mut Enc<'_>, out: &mut Vec<u8>, s: StateMark) {
    match s {
        StateMark::None => out.push(0),
        StateMark::Xmin(v) => e.tagged(out, 1, |_, o| put_var(o, u64::from(v))),
        StateMark::Modified(v) => e.tagged(out, 2, |_, o| put_i64(o, v)),
        StateMark::Created(v) => e.tagged(out, 3, |_, o| put_i64(o, v)),
        StateMark::SchemaVersion(v) => e.tagged(out, 4, |_, o| put_var(o, u64::from(v))),
        StateMark::Hash(v) => e.tagged(out, 5, |_, o| put_var(o, v)),
    }
}

fn put_ref<'a>(e: &mut Enc<'a>, out: &mut Vec<u8>, r: &'a Ref) {
    e.record(out, |e, o| {
        put_var(o, u64::from(r.kind.code()));
        e.strs(o, &r.path);
        put_server_id(e, o, r.server_id);
    });
}

fn put_opt_ref<'a>(e: &mut Enc<'a>, out: &mut Vec<u8>, r: Option<&'a Ref>) {
    match r {
        None => out.push(0),
        Some(r) => {
            out.push(1);
            put_ref(e, out, r);
        }
    }
}

fn put_watermark<'a>(e: &mut Enc<'a>, out: &mut Vec<u8>, w: &'a Watermark) {
    match w {
        Watermark::None => out.push(0),
        Watermark::PgXid { xid, system_id } => e.tagged(out, 1, |_, o| {
            put_var(o, *xid);
            put_var(o, *system_id);
        }),
        Watermark::MyTime {
            at_utc_micros,
            server_uuid,
        } => e.tagged(out, 2, |e, o| {
            put_i64(o, *at_utc_micros);
            e.opt_str(o, server_uuid.as_ref());
        }),
        Watermark::SqliteSchema { version, digest } => e.tagged(out, 3, |_, o| {
            put_var(o, u64::from(*version));
            put_var(o, *digest);
        }),
    }
}

fn fold_code(r: FoldRule) -> u8 {
    match r {
        FoldRule::Exact => 0,
        FoldRule::AsciiCaseless => 1,
    }
}

fn put_props<'a>(e: &mut Enc<'a>, out: &mut Vec<u8>, p: &'a Props) {
    match p {
        Props::None => out.push(0),
        Props::Database(p) => e.tagged(out, 1, |e, o| {
            e.opt_str(o, p.owner.as_ref());
            e.opt_str(o, p.encoding.as_ref());
            e.opt_str(o, p.collation.as_ref());
            e.opt_str(o, p.ctype.as_ref());
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Schema(p) => e.tagged(out, 2, |e, o| {
            e.opt_str(o, p.owner.as_ref());
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Table(p) => e.tagged(out, 3, |e, o| {
            e.opt_str(o, p.owner.as_ref());
            e.opt_str(o, p.comment.as_ref());
            put_opt_ref(e, o, p.partition_of.as_ref());
            e.opt_str(o, p.partition_key.as_ref());
            e.opt_str(o, p.partition_bound.as_ref());
            put_opt_i64(o, p.row_estimate);
            match &p.ext {
                TableExt::None => o.push(0),
                TableExt::Pg(x) => e.tagged(o, 1, |e, o| {
                    o.push(x.relkind);
                    o.push(x.persistence);
                    e.opt_str(o, x.tablespace.as_ref());
                    e.opt_str(o, x.access_method.as_ref());
                    e.strs(o, &x.options);
                }),
                TableExt::MySql(x) => e.tagged(o, 2, |e, o| {
                    e.opt_str(o, x.engine.as_ref());
                    e.opt_str(o, x.row_format.as_ref());
                    e.opt_str(o, x.collation.as_ref());
                    put_opt_u64(o, x.auto_increment);
                    put_opt_i64(o, x.create_time);
                }),
                TableExt::Sqlite(x) => e.tagged(o, 3, |e, o| {
                    put_bool(o, x.without_rowid);
                    put_bool(o, x.strict);
                    e.opt_str(o, x.virtual_module.as_ref());
                }),
            }
        }),
        Props::View(p) => e.tagged(out, 4, |e, o| {
            e.opt_str(o, p.owner.as_ref());
            e.opt_str(o, p.comment.as_ref());
            put_source(o, p.source);
            e.opt_str(o, p.check_option.as_ref());
            put_opt_bool(o, p.updatable);
            match &p.ext {
                ViewExt::None => o.push(0),
                ViewExt::MySql(x) => e.tagged(o, 1, |e, o| {
                    e.opt_str(o, x.algorithm.as_ref());
                    e.opt_str(o, x.definer.as_ref());
                    e.opt_str(o, x.sql_security.as_ref());
                }),
            }
        }),
        Props::Column(p) => e.tagged(out, 5, |e, o| {
            put_var(o, u64::from(p.ordinal));
            e.str(o, &p.type_text);
            put_bool(o, p.nullable);
            e.opt_str(o, p.default_expr.as_ref());
            o.push(match p.generated {
                Generated::No => 0,
                Generated::Stored => 1,
                Generated::Virtual => 2,
            });
            e.opt_str(o, p.generation_expr.as_ref());
            o.push(match p.identity {
                Identity::No => 0,
                Identity::Always => 1,
                Identity::ByDefault => 2,
                Identity::AutoIncrement => 3,
            });
            e.opt_str(o, p.collation.as_ref());
            e.opt_str(o, p.comment.as_ref());
            match &p.ext {
                ColumnExt::None => o.push(0),
                ColumnExt::Pg(x) => e.tagged(o, 1, |e, o| {
                    put_i64(o, i64::from(x.attnum));
                    o.push(x.storage.unwrap_or(0));
                    e.opt_str(o, x.compression.as_ref());
                }),
                ColumnExt::MySql(x) => e.tagged(o, 2, |e, o| {
                    e.opt_str(o, x.column_type.as_ref());
                    e.opt_str(o, x.extra.as_ref());
                    e.opt_str(o, x.charset.as_ref());
                }),
                ColumnExt::Sqlite(x) => e.tagged(o, 3, |_, o| o.push(x.hidden)),
            }
        }),
        Props::Index(p) => e.tagged(out, 6, |e, o| {
            put_bool(o, p.unique);
            put_bool(o, p.primary);
            e.opt_str(o, p.method.as_ref());
            put_var(o, p.parts.len() as u64);
            for part in &p.parts {
                e.record(o, |e, o| {
                    e.opt_str(o, part.column.as_ref());
                    e.opt_str(o, part.expr.as_ref());
                    put_bool(o, part.desc);
                    put_opt_bool(o, part.nulls_first);
                    put_opt_u64(o, part.prefix_len.map(u64::from));
                });
            }
            e.strs(o, &p.include);
            e.opt_str(o, p.predicate.as_ref());
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Key(p) => e.tagged(out, 7, |e, o| {
            e.strs(o, &p.columns);
            e.opt_str(o, p.index_name.as_ref());
            o.push(deferrable_code(p.deferrable));
        }),
        Props::ForeignKey(p) => e.tagged(out, 8, |e, o| {
            e.strs(o, &p.columns);
            put_ref(e, o, &p.target);
            e.strs(o, &p.target_columns);
            o.push(fk_action_code(p.on_update));
            o.push(fk_action_code(p.on_delete));
            o.push(match p.match_kind {
                FkMatch::Simple => 0,
                FkMatch::Full => 1,
                FkMatch::Partial => 2,
            });
            o.push(deferrable_code(p.deferrable));
        }),
        Props::Check(p) => e.tagged(out, 9, |e, o| e.str(o, &p.expr)),
        Props::Routine(p) => e.tagged(out, 10, |e, o| {
            o.push(match p.kind {
                RoutineKind::Function => 0,
                RoutineKind::Procedure => 1,
                RoutineKind::Aggregate => 2,
                RoutineKind::Window => 3,
            });
            put_var(o, p.args.len() as u64);
            for a in &p.args {
                e.record(o, |e, o| {
                    e.opt_str(o, a.name.as_ref());
                    o.push(match a.mode {
                        ArgMode::In => 0,
                        ArgMode::Out => 1,
                        ArgMode::InOut => 2,
                        ArgMode::Variadic => 3,
                        ArgMode::Table => 4,
                    });
                    e.str(o, &a.type_text);
                    e.opt_str(o, a.default_expr.as_ref());
                });
            }
            e.opt_str(o, p.returns.as_ref());
            put_bool(o, p.returns_set);
            e.opt_str(o, p.language.as_ref());
            o.push(match p.volatility {
                Volatility::Unknown => 0,
                Volatility::Immutable => 1,
                Volatility::Stable => 2,
                Volatility::Volatile => 3,
            });
            put_bool(o, p.security_definer);
            e.opt_str(o, p.overload_sig.as_ref());
            put_source(o, p.source);
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Sequence(p) => e.tagged(out, 11, |e, o| {
            e.opt_str(o, p.data_type.as_ref());
            put_opt_i64(o, p.start);
            put_opt_i64(o, p.increment);
            put_opt_i64(o, p.min);
            put_opt_i64(o, p.max);
            put_opt_i64(o, p.cache);
            put_bool(o, p.cycle);
            match &p.owned_by {
                None => o.push(0),
                Some((r, col)) => {
                    o.push(1);
                    put_ref(e, o, r);
                    e.str(o, col);
                }
            }
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Trigger(p) => e.tagged(out, 12, |e, o| {
            o.push(match p.timing {
                TriggerTiming::Before => 0,
                TriggerTiming::After => 1,
                TriggerTiming::InsteadOf => 2,
            });
            o.push(p.events.bits());
            put_bool(o, p.for_each_row);
            put_opt_ref(e, o, p.function.as_ref());
            e.opt_str(o, p.when_expr.as_ref());
            put_source(o, p.source);
            put_bool(o, p.enabled);
        }),
        Props::Type(p) => e.tagged(out, 13, |e, o| {
            e.opt_str(o, p.base.as_ref());
            e.strs(o, &p.labels);
            put_bool(o, p.not_null);
            e.opt_str(o, p.default_expr.as_ref());
            e.opt_str(o, p.comment.as_ref());
        }),
        Props::Generic(p) => e.tagged(out, 14, |e, o| {
            e.opt_str(o, p.owner.as_ref());
            e.opt_str(o, p.version.as_ref());
            e.opt_str(o, p.comment.as_ref());
        }),
    }
}

fn deferrable_code(d: Deferrable) -> u8 {
    match d {
        Deferrable::No => 0,
        Deferrable::InitiallyImmediate => 1,
        Deferrable::InitiallyDeferred => 2,
    }
}

fn fk_action_code(a: FkAction) -> u8 {
    match a {
        FkAction::NoAction => 0,
        FkAction::Restrict => 1,
        FkAction::Cascade => 2,
        FkAction::SetNull => 3,
        FkAction::SetDefault => 4,
    }
}

fn put_obj<'a>(e: &mut Enc<'a>, out: &mut Vec<u8>, o: &'a Obj) {
    e.record(out, |e, b| {
        put_var(b, u64::from(o.kind.code()));
        e.str(b, &o.name);
        put_var(
            b,
            if o.parent == NO_PARENT {
                0
            } else {
                u64::from(o.parent) + 1
            },
        );
        put_var(b, u64::from(o.flags.bits()));
        put_server_id(e, b, o.server_id);
        put_state(e, b, o.state);
        put_props(e, b, &o.props);
    });
}

/// Encodes `ns` at the current format version.
pub fn encode_namespace(ns: &NamespaceSnapshot) -> Vec<u8> {
    encode_with(ns, FORMAT_MINOR, &[])
}

/// Encodes with an explicit minor and `pad` appended to every object record: the tests'
/// stand-in for a newer writer that added trailing fields.
fn encode_with(ns: &NamespaceSnapshot, minor: u16, pad: &[u8]) -> Vec<u8> {
    let mut e = Enc {
        index: HashMap::new(),
        list: Vec::new(),
    };
    let mut tail = Vec::with_capacity(ns.len() * 32);
    let key = ns.key();
    let dbms_id = ns.dbms().id();
    e.record(&mut tail, |e, o| {
        o.push(key.kind.code());
        e.strs(o, &key.path);
        e.str(o, dbms_id);
        o.push(fold_code(ns.fold().relations));
        o.push(fold_code(ns.fold().other));
        o.push(ns.meta().level.code());
        put_watermark(e, o, &ns.meta().watermark);
        put_i64(o, ns.meta().loaded_at_ms);
    });
    put_var(&mut tail, ns.len() as u64);
    for o in ns.objs() {
        if pad.is_empty() {
            put_obj(&mut e, &mut tail, o);
        } else {
            let mut rec = Vec::new();
            put_obj(&mut e, &mut rec, o);
            let mut d = Dec {
                b: &rec,
                pos: 0,
                strings: &[],
            };
            let len = d.var().unwrap_or(0) as usize;
            let mut body = rec[d.pos..d.pos + len].to_vec();
            body.extend_from_slice(pad);
            put_var(&mut tail, body.len() as u64);
            tail.extend_from_slice(&body);
        }
    }
    let mut body = Vec::with_capacity(tail.len() + e.list.len() * 12 + 8);
    put_var(&mut body, e.list.len() as u64);
    for s in &e.list {
        put_var(&mut body, s.len() as u64);
        body.extend_from_slice(s.as_bytes());
    }
    body.extend_from_slice(&tail);

    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    out.extend_from_slice(&minor.to_le_bytes());
    out.push(COMPRESSION_NONE);
    out.extend_from_slice(&twox_hash::XxHash3_64::oneshot(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The canonical bytes of a namespace key (the cache's primary key).
pub fn encode_ns_key(key: &NsKey) -> Vec<u8> {
    let mut out = vec![key.kind.code()];
    for p in &key.path {
        put_var(&mut out, p.len() as u64);
        out.extend_from_slice(p.as_bytes());
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Decoding.

struct Dec<'a> {
    b: &'a [u8],
    pos: usize,
    strings: &'a [Name],
}

impl<'a> Dec<'a> {
    fn more(&self) -> bool {
        self.pos < self.b.len()
    }

    fn u8(&mut self) -> DResult<u8> {
        let v = *self.b.get(self.pos).ok_or(DecodeError::Truncated)?;
        self.pos += 1;
        Ok(v)
    }

    fn var(&mut self) -> DResult<u64> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if shift == 63 && byte > 1 {
                    return Err(DecodeError::VarintOverflow);
                }
                return Ok(v);
            }
        }
        Err(DecodeError::VarintOverflow)
    }

    fn u32(&mut self) -> DResult<u32> {
        u32::try_from(self.var()?).map_err(|_| DecodeError::VarintOverflow)
    }

    fn i64(&mut self) -> DResult<i64> {
        Ok(unzigzag(self.var()?))
    }

    fn bool(&mut self) -> DResult<bool> {
        Ok(self.u8()? != 0)
    }

    fn bytes(&mut self, n: usize) -> DResult<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.b.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    fn len(&mut self) -> DResult<usize> {
        usize::try_from(self.var()?).map_err(|_| DecodeError::VarintOverflow)
    }

    /// A length-prefixed record as its own reader.
    fn record(&mut self) -> DResult<Dec<'a>> {
        let n = self.len()?;
        Ok(Dec {
            b: self.bytes(n)?,
            pos: 0,
            strings: self.strings,
        })
    }

    /// A tagged union value: `None` for tag 0.
    fn tagged(&mut self) -> DResult<Option<(u64, Dec<'a>)>> {
        match self.var()? {
            0 => Ok(None),
            tag => Ok(Some((tag, self.record()?))),
        }
    }

    fn str(&mut self) -> DResult<Name> {
        let i = self.len()?;
        self.strings
            .get(i)
            .cloned()
            .ok_or(DecodeError::BadStringIndex)
    }

    fn opt_str(&mut self) -> DResult<Option<Name>> {
        match self.len()? {
            0 => Ok(None),
            i => self
                .strings
                .get(i - 1)
                .cloned()
                .map(Some)
                .ok_or(DecodeError::BadStringIndex),
        }
    }

    /// A count that cannot exceed the bytes left (one byte per element at least), so a
    /// corrupt count cannot trigger a huge allocation.
    fn count(&mut self) -> DResult<usize> {
        let n = self.len()?;
        if n > self.b.len() - self.pos {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }

    fn strs(&mut self) -> DResult<Vec<Name>> {
        let n = self.count()?;
        (0..n).map(|_| self.str()).collect()
    }

    fn opt_i64(&mut self) -> DResult<Option<i64>> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.i64()?),
        })
    }

    fn opt_u64(&mut self) -> DResult<Option<u64>> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.var()?),
        })
    }

    fn opt_bool(&mut self) -> DResult<Option<bool>> {
        Ok(match self.u8()? {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        })
    }

    fn source(&mut self) -> DResult<Option<SourceRef>> {
        Ok(match self.u8()? {
            0 => None,
            _ => {
                let mut h = [0u8; 16];
                h.copy_from_slice(self.bytes(16)?);
                Some(SourceRef {
                    hash: u128::from_le_bytes(h),
                    len: self.u32()?,
                })
            }
        })
    }

    fn server_id(&mut self) -> DResult<Option<ServerId>> {
        let Some((tag, mut r)) = self.tagged()? else {
            return Ok(None);
        };
        Ok(match tag {
            1 => Some(ServerId::Oid(r.u32()?)),
            2 => Some(ServerId::ObjectId(
                i32::try_from(r.i64()?).map_err(|_| DecodeError::VarintOverflow)?,
            )),
            3 => Some(ServerId::ObjNum(r.var()?)),
            4 => {
                let mut u = [0u8; 16];
                u.copy_from_slice(r.bytes(16)?);
                Some(ServerId::Uuid(u))
            }
            5 => Some(ServerId::RowId(r.i64()?)),
            _ => None,
        })
    }

    fn state(&mut self) -> DResult<StateMark> {
        let Some((tag, mut r)) = self.tagged()? else {
            return Ok(StateMark::None);
        };
        Ok(match tag {
            1 => StateMark::Xmin(r.u32()?),
            2 => StateMark::Modified(r.i64()?),
            3 => StateMark::Created(r.i64()?),
            4 => StateMark::SchemaVersion(r.u32()?),
            5 => StateMark::Hash(r.var()?),
            _ => StateMark::None,
        })
    }

    fn kind(&mut self) -> DResult<ObjectKind> {
        let code = u16::try_from(self.var()?).unwrap_or(u16::MAX);
        Ok(ObjectKind::from_code(code).unwrap_or(ObjectKind::Unknown))
    }

    fn reference(&mut self) -> DResult<Ref> {
        let mut r = self.record()?;
        Ok(Ref {
            kind: r.kind()?,
            path: r.strs()?.into_iter().collect::<SmallVec<_>>(),
            server_id: r.server_id()?,
        })
    }

    fn opt_ref(&mut self) -> DResult<Option<Ref>> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.reference()?),
        })
    }

    fn watermark(&mut self) -> DResult<Watermark> {
        let Some((tag, mut r)) = self.tagged()? else {
            return Ok(Watermark::None);
        };
        Ok(match tag {
            1 => Watermark::PgXid {
                xid: r.var()?,
                system_id: r.var()?,
            },
            2 => Watermark::MyTime {
                at_utc_micros: r.i64()?,
                server_uuid: r.opt_str()?,
            },
            3 => Watermark::SqliteSchema {
                version: r.u32()?,
                digest: r.var()?,
            },
            _ => Watermark::None,
        })
    }

    fn props(&mut self) -> DResult<Props> {
        let Some((tag, mut r)) = self.tagged()? else {
            return Ok(Props::None);
        };
        let r = &mut r;
        Ok(match tag {
            1 => Props::Database(Arc::new(DatabaseProps {
                owner: r.opt_str()?,
                encoding: r.opt_str()?,
                collation: r.opt_str()?,
                ctype: r.opt_str()?,
                comment: r.opt_str()?,
            })),
            2 => Props::Schema(Arc::new(SchemaProps {
                owner: r.opt_str()?,
                comment: r.opt_str()?,
            })),
            3 => Props::Table(Arc::new(TableProps {
                owner: r.opt_str()?,
                comment: r.opt_str()?,
                partition_of: r.opt_ref()?,
                partition_key: r.opt_str()?,
                partition_bound: r.opt_str()?,
                row_estimate: r.opt_i64()?,
                ext: match r.tagged()? {
                    None => TableExt::None,
                    Some((1, mut x)) => TableExt::Pg(PgTableExt {
                        relkind: x.u8()?,
                        persistence: x.u8()?,
                        tablespace: x.opt_str()?,
                        access_method: x.opt_str()?,
                        options: x.strs()?,
                    }),
                    Some((2, mut x)) => TableExt::MySql(MySqlTableExt {
                        engine: x.opt_str()?,
                        row_format: x.opt_str()?,
                        collation: x.opt_str()?,
                        auto_increment: x.opt_u64()?,
                        create_time: x.opt_i64()?,
                    }),
                    Some((3, mut x)) => TableExt::Sqlite(SqliteTableExt {
                        without_rowid: x.bool()?,
                        strict: x.bool()?,
                        virtual_module: x.opt_str()?,
                    }),
                    Some(_) => TableExt::None,
                },
            })),
            4 => Props::View(Arc::new(ViewProps {
                owner: r.opt_str()?,
                comment: r.opt_str()?,
                source: r.source()?,
                check_option: r.opt_str()?,
                updatable: r.opt_bool()?,
                ext: match r.tagged()? {
                    Some((1, mut x)) => ViewExt::MySql(MySqlViewExt {
                        algorithm: x.opt_str()?,
                        definer: x.opt_str()?,
                        sql_security: x.opt_str()?,
                    }),
                    _ => ViewExt::None,
                },
            })),
            5 => Props::Column(Arc::new(ColumnProps {
                ordinal: r.u32()?,
                type_text: r.str()?,
                nullable: r.bool()?,
                default_expr: r.opt_str()?,
                generated: match r.u8()? {
                    1 => Generated::Stored,
                    2 => Generated::Virtual,
                    _ => Generated::No,
                },
                generation_expr: r.opt_str()?,
                identity: match r.u8()? {
                    1 => Identity::Always,
                    2 => Identity::ByDefault,
                    3 => Identity::AutoIncrement,
                    _ => Identity::No,
                },
                collation: r.opt_str()?,
                comment: r.opt_str()?,
                ext: match r.tagged()? {
                    Some((1, mut x)) => ColumnExt::Pg(PgColumnExt {
                        attnum: i16::try_from(x.i64()?).map_err(|_| DecodeError::VarintOverflow)?,
                        storage: Some(x.u8()?).filter(|&s| s != 0),
                        compression: x.opt_str()?,
                    }),
                    Some((2, mut x)) => ColumnExt::MySql(MySqlColumnExt {
                        column_type: x.opt_str()?,
                        extra: x.opt_str()?,
                        charset: x.opt_str()?,
                    }),
                    Some((3, mut x)) => ColumnExt::Sqlite(SqliteColumnExt { hidden: x.u8()? }),
                    _ => ColumnExt::None,
                },
            })),
            6 => {
                let unique = r.bool()?;
                let primary = r.bool()?;
                let method = r.opt_str()?;
                let n = r.count()?;
                let mut parts = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut p = r.record()?;
                    parts.push(IndexPart {
                        column: p.opt_str()?,
                        expr: p.opt_str()?,
                        desc: p.bool()?,
                        nulls_first: p.opt_bool()?,
                        prefix_len: p
                            .opt_u64()?
                            .map(u32::try_from)
                            .transpose()
                            .map_err(|_| DecodeError::VarintOverflow)?,
                    });
                }
                Props::Index(Arc::new(IndexProps {
                    unique,
                    primary,
                    method,
                    parts,
                    include: r.strs()?,
                    predicate: r.opt_str()?,
                    comment: r.opt_str()?,
                }))
            }
            7 => Props::Key(Arc::new(KeyProps {
                columns: r.strs()?,
                index_name: r.opt_str()?,
                deferrable: deferrable(r.u8()?),
            })),
            8 => Props::ForeignKey(Arc::new(ForeignKeyProps {
                columns: r.strs()?,
                target: r.reference()?,
                target_columns: r.strs()?,
                on_update: fk_action(r.u8()?),
                on_delete: fk_action(r.u8()?),
                match_kind: match r.u8()? {
                    1 => FkMatch::Full,
                    2 => FkMatch::Partial,
                    _ => FkMatch::Simple,
                },
                deferrable: deferrable(r.u8()?),
            })),
            9 => Props::Check(Arc::new(CheckProps { expr: r.str()? })),
            10 => {
                let kind = match r.u8()? {
                    1 => RoutineKind::Procedure,
                    2 => RoutineKind::Aggregate,
                    3 => RoutineKind::Window,
                    _ => RoutineKind::Function,
                };
                let n = r.count()?;
                let mut args = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut a = r.record()?;
                    args.push(Arg {
                        name: a.opt_str()?,
                        mode: match a.u8()? {
                            1 => ArgMode::Out,
                            2 => ArgMode::InOut,
                            3 => ArgMode::Variadic,
                            4 => ArgMode::Table,
                            _ => ArgMode::In,
                        },
                        type_text: a.str()?,
                        default_expr: a.opt_str()?,
                    });
                }
                Props::Routine(Arc::new(RoutineProps {
                    kind,
                    args,
                    returns: r.opt_str()?,
                    returns_set: r.bool()?,
                    language: r.opt_str()?,
                    volatility: match r.u8()? {
                        1 => Volatility::Immutable,
                        2 => Volatility::Stable,
                        3 => Volatility::Volatile,
                        _ => Volatility::Unknown,
                    },
                    security_definer: r.bool()?,
                    overload_sig: r.opt_str()?,
                    source: r.source()?,
                    comment: r.opt_str()?,
                }))
            }
            11 => Props::Sequence(Arc::new(SequenceProps {
                data_type: r.opt_str()?,
                start: r.opt_i64()?,
                increment: r.opt_i64()?,
                min: r.opt_i64()?,
                max: r.opt_i64()?,
                cache: r.opt_i64()?,
                cycle: r.bool()?,
                owned_by: match r.u8()? {
                    0 => None,
                    _ => Some((r.reference()?, r.str()?)),
                },
                comment: r.opt_str()?,
            })),
            12 => Props::Trigger(Arc::new(TriggerProps {
                timing: match r.u8()? {
                    1 => TriggerTiming::After,
                    2 => TriggerTiming::InsteadOf,
                    _ => TriggerTiming::Before,
                },
                events: TriggerEvents::from_bits_truncate(r.u8()?),
                for_each_row: r.bool()?,
                function: r.opt_ref()?,
                when_expr: r.opt_str()?,
                source: r.source()?,
                enabled: r.bool()?,
            })),
            13 => Props::Type(Arc::new(TypeProps {
                base: r.opt_str()?,
                labels: r.strs()?,
                not_null: r.bool()?,
                default_expr: r.opt_str()?,
                comment: r.opt_str()?,
            })),
            14 => Props::Generic(Arc::new(GenericProps {
                owner: r.opt_str()?,
                version: r.opt_str()?,
                comment: r.opt_str()?,
            })),
            _ => Props::None,
        })
    }

    fn obj(&mut self) -> DResult<Obj> {
        let mut r = self.record()?;
        let kind = r.kind()?;
        let name = r.str()?;
        let parent = match r.var()? {
            0 => NO_PARENT,
            p => u32::try_from(p - 1).map_err(|_| DecodeError::VarintOverflow)?,
        };
        let flags = ObjFlags::from_bits_retain(r.u32()?);
        let mut o = Obj::new(kind, name).with_flags(flags);
        o.server_id = r.server_id()?;
        o.state = r.state()?;
        o.props = r.props()?;
        o.parent = parent;
        Ok(o)
    }
}

fn deferrable(c: u8) -> Deferrable {
    match c {
        1 => Deferrable::InitiallyImmediate,
        2 => Deferrable::InitiallyDeferred,
        _ => Deferrable::No,
    }
}

fn fk_action(c: u8) -> FkAction {
    match c {
        1 => FkAction::Restrict,
        2 => FkAction::Cascade,
        3 => FkAction::SetNull,
        4 => FkAction::SetDefault,
        _ => FkAction::NoAction,
    }
}

fn fold_rule(c: u8) -> FoldRule {
    match c {
        1 => FoldRule::AsciiCaseless,
        _ => FoldRule::Exact,
    }
}

/// The format version of a payload, without decoding it.
pub fn payload_version(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
        return None;
    }
    Some((
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
    ))
}

/// Decodes a payload written by [`encode_namespace`] (any minor of [`FORMAT_MAJOR`]),
/// validates its layout and rebuilds its indexes.
pub fn decode_namespace(bytes: &[u8]) -> Result<NamespaceSnapshot, DecodeError> {
    let (major, minor) = payload_version(bytes).ok_or(DecodeError::BadMagic)?;
    if major != FORMAT_MAJOR {
        return Err(DecodeError::Major(major));
    }
    match bytes[8] {
        COMPRESSION_NONE => {}
        c => return Err(DecodeError::UnknownCompression(c)),
    }
    let mut sum = [0u8; 8];
    sum.copy_from_slice(&bytes[9..HEADER_LEN]);
    let body = &bytes[HEADER_LEN..];
    if twox_hash::XxHash3_64::oneshot(body) != u64::from_le_bytes(sum) {
        return Err(DecodeError::Checksum);
    }

    let mut d = Dec {
        b: body,
        pos: 0,
        strings: &[],
    };
    let n = d.count()?;
    let mut strings: Vec<Name> = Vec::with_capacity(n);
    for _ in 0..n {
        let len = d.len()?;
        let s = std::str::from_utf8(d.bytes(len)?).map_err(|_| DecodeError::BadUtf8)?;
        strings.push(Arc::from(s));
    }
    let mut d = Dec {
        b: body,
        pos: d.pos,
        strings: &strings,
    };

    let mut h = d.record()?;
    let ns_kind = NsKind::from_code(h.u8()?).ok_or(DecodeError::BadNsHeader)?;
    let path = h.strs()?.into_iter().collect();
    let dbms = DbmsKind::from_id(&h.str()?).ok_or(DecodeError::UnknownDbms)?;
    let fold = FoldPolicy {
        relations: fold_rule(h.u8()?),
        other: fold_rule(h.u8()?),
    };
    let level = Level::from_code(h.u8()?).unwrap_or_default();
    let watermark = h.watermark()?;
    let loaded_at_ms = h.i64()?;
    let key = NsKey {
        kind: ns_kind,
        path,
    };

    let count = d.count()?;
    let mut objs = Vec::with_capacity(count);
    for _ in 0..count {
        objs.push(d.obj()?);
    }
    // A newer minor may append sections after the objects (ignored); ours never does.
    let newer = minor > FORMAT_MINOR;
    if d.more() && !newer {
        return Err(DecodeError::TrailingBytes);
    }
    let meta = NsMeta {
        level,
        watermark,
        loaded_at_ms,
    };
    let mut ns = NamespaceSnapshot::from_arena(key, dbms, fold, meta, objs, newer)
        .map_err(DecodeError::Model)?;
    migrate_minor(&mut ns, minor);
    Ok(ns)
}

/// The minor-version migration chain (HV-MODEL-03 step 3): each step is pure and tested.
/// Format 1.0 is the first minor, so there is nothing to migrate yet; a 1.1 step would read
/// `if from < 1 { … }`.
fn migrate_minor(_ns: &mut NamespaceSnapshot, _from: u16) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace::Subtree;

    fn tiny() -> NamespaceSnapshot {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
        let tab = t.add(
            t.root(),
            Obj::new(ObjectKind::Table, "t")
                .with_server_id(ServerId::Oid(16400))
                .with_props(Props::Table(Arc::new(TableProps::default()))),
        );
        t.add(
            tab,
            Obj::new(ObjectKind::Column, "a").with_props(Props::Column(Arc::new(ColumnProps {
                ordinal: 1,
                type_text: Arc::from("integer"),
                ..Default::default()
            }))),
        );
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            DbmsKind::Postgres,
            FoldPolicy::for_dbms(DbmsKind::Postgres),
            NsMeta::default(),
            t,
        )
        .unwrap()
    }

    #[test]
    fn varints_round_trip_at_the_edges() {
        for v in [
            0,
            1,
            127,
            128,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX,
        ] {
            let mut b = Vec::new();
            put_var(&mut b, v);
            let mut d = Dec {
                b: &b,
                pos: 0,
                strings: &[],
            };
            assert_eq!(d.var().unwrap(), v);
            assert!(!d.more());
        }
        for v in [0, -1, 1, i64::MIN, i64::MAX] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn a_newer_minor_with_trailing_fields_decodes_and_is_marked() {
        let ns = tiny();
        let bytes = encode_with(&ns, FORMAT_MINOR + 1, &[0xAB, 0xCD, 0x01]);
        let back = decode_namespace(&bytes).unwrap();
        assert!(back.from_newer_minor());
        assert_eq!(back.objs(), ns.objs());
    }

    #[test]
    fn an_unknown_union_tag_is_skipped_and_decodes_as_its_default() {
        // Props tag 99 (unknown) with a 2-byte record, then the next field (7).
        let mut d = Dec {
            b: &[99, 2, 1, 2, 7],
            pos: 0,
            strings: &[],
        };
        assert_eq!(d.props().unwrap(), Props::None);
        assert_eq!(d.u8().unwrap(), 7);
        let mut d = Dec {
            b: &[42, 1, 0, 7],
            pos: 0,
            strings: &[],
        };
        assert_eq!(d.watermark().unwrap(), Watermark::None);
        assert_eq!(d.u8().unwrap(), 7);
    }

    #[test]
    fn an_unknown_kind_code_decodes_as_unknown() {
        let mut k = Dec {
            b: &[0xE7, 0x07],
            pos: 0,
            strings: &[],
        };
        assert_eq!(k.kind().unwrap(), ObjectKind::Unknown);
    }

    #[test]
    fn another_major_is_rejected() {
        let mut bytes = encode_namespace(&tiny());
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(decode_namespace(&bytes).unwrap_err(), DecodeError::Major(2));
    }

    #[test]
    fn a_flipped_bit_fails_the_checksum() {
        let mut bytes = encode_namespace(&tiny());
        let last = bytes.len() - 1;
        bytes[last] ^= 0x10;
        assert_eq!(decode_namespace(&bytes).unwrap_err(), DecodeError::Checksum);
    }

    #[test]
    fn every_truncation_is_an_error_never_a_panic() {
        let bytes = encode_namespace(&tiny());
        for cut in 0..bytes.len() {
            assert!(decode_namespace(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        // Past the checksum too: a truncated body with a matching checksum.
        for cut in HEADER_LEN..bytes.len() {
            let mut b = bytes[..cut].to_vec();
            let sum = twox_hash::XxHash3_64::oneshot(&b[HEADER_LEN..]).to_le_bytes();
            b[9..HEADER_LEN].copy_from_slice(&sum);
            assert!(decode_namespace(&b).is_err(), "body cut at {cut}");
        }
    }

    #[test]
    fn corrupt_bodies_with_a_matching_checksum_never_panic() {
        let bytes = encode_namespace(&tiny());
        for at in HEADER_LEN..bytes.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut b = bytes.clone();
                b[at] ^= flip;
                let sum = twox_hash::XxHash3_64::oneshot(&b[HEADER_LEN..]).to_le_bytes();
                b[9..HEADER_LEN].copy_from_slice(&sum);
                let _ = decode_namespace(&b);
            }
        }
    }
}
