//! The versioned binary encoding of a namespace, the cache row payload (HV-MODEL-03; R-M4-26).
//! M4 Task 11, spike origin `d86313b` (the spike's codec, extended to the M4 types).
//!
//! ```text
//! envelope  = "HVNS" major:u16le minor:u16le compression:u8 checksum:u64le body
//! body      = strings ns_header:record count:varint obj:record* [newer-minor sections]
//! strings   = count:varint (len:varint utf8)*          every string of the namespace, once
//! record    = len:varint bytes                          fields in order
//! tagged    = 0 | tag:varint record                     a union value (props, ids, extensions)
//! optrec    = 0 | 1 record                              an optional group of fields
//! ```
//!
//! - **Versioning** (alg/02 HV-MODEL-03 "Versioning"): a *major* bump means a new cache file
//!   (`<profile>.v<major>.sqlite`), so another major is rejected here. A *minor* bump is
//!   additive: new fields are appended at the end of a record, new union tags and new sections
//!   after the objects are added. A reader of an older minor skips what it does not know (every
//!   record is length-prefixed) and maps an unknown union tag to its default (`Props::None`,
//!   `ObjectKind::Unknown`, `Watermark::None`); a namespace decoded from a **newer** minor is
//!   reported [`Decoded::from_newer_minor`], loads with `Watermark::None` (so its next pass is a
//!   full introspection, never an incremental one on top of fields this build dropped) and is
//!   never written back (`persist`). A reader of a **newer** minor reads an older minor's
//!   missing trailing fields as their defaults (a field added in minor `n` is read only when
//!   the record still has bytes) and runs [`migrate_minor`].
//! - **Strictness:** a payload of this build's minor or an older one must use every byte of
//!   every record and of the body; leftovers are an error (corruption), not a skip.
//! - **Unknown kinds:** the namespace header lists the machine id of every kind code it uses.
//!   A code this build does not know decodes as `ObjectKind::Unknown` whose raw kind
//!   (`Props::Generic::raw_kind`) is that id, so it matches the `ObjectKey::raw_kind` a newer
//!   build wrote into user data (T3 review m6), and two unknown kinds stay two kinds.
//! - **Strings** are interned per namespace: the 160k columns of a large schema share a few
//!   dozen type texts and a few thousand column names; the decoder makes one `Arc<str>` per
//!   distinct string.
//! - **Integrity:** XXH3-64 over the body, so a flipped bit drops that row only (edge 12).
//!   Encryption (sensitive profiles) wraps the whole envelope, in `persist`.
//! - **Untrusted input:** decoding never panics, never recurses on input depth, and allocates
//!   at most a small constant times the input length: every count is capped by the bytes left
//!   (divided by the smallest encoding of one element). Its CPU is linear in the input too
//!   (fix round 1, B-I3): a name is at most [`MAX_NAME_BYTES`], and the string bytes a payload
//!   expands to through its table are capped at [`EXPANSION_FACTOR`] × body +
//!   [`EXPANSION_SLACK`]. The encoder enforces both, so whatever it writes can be read back.
//! - **Compression:** the `compression` byte is `0` (none) in format 1 (R-M4-26); a reader
//!   rejects any other value.
//! - The arena is stored in its breadth-first order with parent indexes; children ranges and
//!   name indexes are rebuilt and the layout re-validated on load.
//! - **Minor 1** (M4 Task 13, R-W2-8; a named exception in Task 11's file): the rare parts of
//!   keys, foreign keys and checks are appended as one optional record at the end of the key,
//!   foreign-key and check records (`KeyRare`: `nulls_first`, `opclass`, `collation`;
//!   `FkRare`: `fk_ref_index`; `CheckRare`: `no_inherit`, `excl_ops`, the exclusion deferral),
//!   written only when the rare part is set. A record that ends before it (a minor-0 payload,
//!   or no rare part) reads `rare: None`, so the payload and the fingerprint of an object
//!   without a rare part are the same as in minor 0. Operator classes,
//!   collations and operators are names (capped at [`MAX_NAME_BYTES`]); every string counts
//!   against the expansion budget; `fk_ref_index` is a reference like any other.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use smallvec::SmallVec;
use sql_catalog::types::{ArgMode, FnArg, FnKind, Volatility};

use crate::ids::{NsKey, NsKind, Ref, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{FoldRule, Name};
use crate::namespace::{
    Level, ModelError, NO_PARENT, NamespaceSnapshot, NsMeta, Obj, ObjFlags, PgIdentity, Watermark,
};
use crate::props::*;

/// The format's major version: a reader refuses another major (the cache is rebuilt).
pub const FORMAT_MAJOR: u16 = 1;
/// The format's minor version: a reader reads newer minors and never rewrites them.
pub const FORMAT_MINOR: u16 = 1;

/// The first minor that writes the R-W2-8 rare parts (M4 Task 13).
const MINOR_RARE: u16 = 1;

const MAGIC: &[u8; 4] = b"HVNS";
/// Bytes before the body: magic, major, minor, compression, checksum.
pub const HEADER_LEN: usize = 4 + 2 + 2 + 1 + 8;
const COMPRESSION_NONE: u8 = 0;

/// The smallest encoding of one object record (length, kind, name, parent, flags, server id,
/// state, props: one byte each); caps the object count a payload can claim.
const MIN_OBJ_BYTES: usize = 8;

/// The longest name a payload may hold, in bytes: object names and every namespace or
/// reference path element (B-I3). The engines' identifier limits are far below it — PostgreSQL
/// 63 bytes, MySQL/MariaDB 64 characters (256 bytes in utf8mb4), SQL Server and Oracle 128
/// characters (512 bytes) — so 4 KiB leaves headroom for the longest identifier of any
/// engine in any encoding; SQLite has no limit, and a SQLite name over the cap is not cached
/// (the encoder refuses it, so no row is written that could not be read).
pub const MAX_NAME_BYTES: usize = 4096;

/// The string bytes one payload may expand to through its string table (every reference
/// counted at its full length; B-I3): this many times the body, plus [`EXPANSION_SLACK`].
/// A real catalog expands to well under its own size (160k columns: below 1×), so 8× is
/// headroom, while a crafted row that references one long string from many 8-byte records
/// fails fast instead of costing O(objects × length).
pub const EXPANSION_FACTOR: usize = 8;
/// The most path elements a namespace key may have (fix round 2, N2): real keys have at most
/// two (`[db, schema]`).
pub const MAX_NS_PATH: usize = 16;
/// The fixed part of the expansion budget (small payloads).
pub const EXPANSION_SLACK: usize = 1 << 20;

/// Why a payload cannot be encoded or decoded. Decoding never panics on any input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Shorter than its header or a length prefix says.
    Truncated,
    /// Not a namespace payload.
    BadMagic,
    /// Another major format.
    Major { found: u16 },
    /// The body's checksum does not match.
    Checksum,
    /// A reserved compression byte this build does not know.
    Compression(u8),
    /// A malformed record.
    Malformed(&'static str),
    /// The objects do not form a valid namespace arena.
    Model(ModelError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "truncated payload"),
            DecodeError::BadMagic => write!(f, "not a namespace payload"),
            DecodeError::Major { found } => write!(f, "format major {found}"),
            DecodeError::Checksum => write!(f, "checksum mismatch"),
            DecodeError::Compression(c) => write!(f, "unknown compression {c}"),
            DecodeError::Malformed(why) => write!(f, "malformed payload: {why}"),
            DecodeError::Model(e) => write!(f, "invalid arena: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

type DResult<T> = Result<T, DecodeError>;

/// A decoded payload and its format facts.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub ns: NamespaceSnapshot,
    /// The minor the payload was written with.
    pub minor: u16,
    /// Written by a newer minor: read, never written back, loaded with `Watermark::None`.
    pub from_newer_minor: bool,
}

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

/// The encoder: the string table (or inline strings) and what to leave out.
pub(crate) struct Enc<'a> {
    index: HashMap<&'a str, u32>,
    list: Vec<&'a str>,
    /// Strings written in place (`len utf8`) instead of as table indexes.
    inline: bool,
    /// Fingerprint form: server ids (also inside references) and state marks are left out.
    canonical: bool,
    /// Tests only: write this kind under another code and id (a newer build's kind).
    remap: Option<(ObjectKind, u16, &'a str)>,
    /// A name over [`MAX_NAME_BYTES`] was written (the payload is refused: it would not
    /// decode).
    long_name: bool,
    /// String bytes the table references expand to (the decoder's budget, B-I3).
    expanded: usize,
    /// The format minor being written: fields added after it are left out (tests write older
    /// minors; M4 Task 13).
    minor: u16,
}

impl<'a> Enc<'a> {
    fn table() -> Self {
        Enc {
            index: HashMap::new(),
            list: Vec::new(),
            inline: false,
            canonical: false,
            remap: None,
            long_name: false,
            expanded: 0,
            minor: FORMAT_MINOR,
        }
    }

    /// A name (an object name, a namespace or reference path element): capped.
    fn name(&mut self, out: &mut Vec<u8>, s: &'a str) {
        self.long_name |= s.len() > MAX_NAME_BYTES;
        self.str(out, s);
    }

    fn names(&mut self, out: &mut Vec<u8>, v: &'a [Name]) {
        put_var(out, v.len() as u64);
        for s in v {
            self.name(out, s);
        }
    }

    fn code(&self, kind: ObjectKind) -> u16 {
        match self.remap {
            Some((k, code, _)) if k == kind => code,
            _ => kind.code(),
        }
    }

    fn inline() -> Self {
        Enc {
            inline: true,
            ..Enc::table()
        }
    }

    /// The canonical (fingerprint) encoder: inline strings, no server ids, no state marks.
    pub(crate) fn canonical() -> Self {
        Enc {
            inline: true,
            canonical: true,
            ..Enc::table()
        }
    }

    fn str(&mut self, out: &mut Vec<u8>, s: &'a str) {
        if self.inline {
            put_var(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
            return;
        }
        self.expanded = self.expanded.saturating_add(s.len());
        let i = match self.index.get(s) {
            Some(&i) => i,
            None => {
                let i = u32::try_from(self.list.len()).unwrap_or(u32::MAX);
                self.list.push(s);
                self.index.insert(s, i);
                i
            }
        };
        put_var(out, u64::from(i));
    }

    fn opt_str(&mut self, out: &mut Vec<u8>, s: Option<&'a Name>) {
        match s {
            None => out.push(0),
            Some(s) => {
                out.push(1);
                self.str(out, s);
            }
        }
    }

    fn strs(&mut self, out: &mut Vec<u8>, v: &'a [Name]) {
        put_var(out, v.len() as u64);
        for s in v {
            self.str(out, s);
        }
    }

    /// A length-prefixed record whose fields `f` writes (in place: the body is written, then
    /// its length is inserted before it, so no buffer is allocated per record).
    fn record(&mut self, out: &mut Vec<u8>, f: impl FnOnce(&mut Self, &mut Vec<u8>)) {
        let start = out.len();
        f(self, out);
        let mut len = (out.len() - start) as u64;
        let mut hdr = [0u8; 10];
        let mut n = 0;
        while len >= 0x80 {
            hdr[n] = (len as u8) | 0x80;
            len >>= 7;
            n += 1;
        }
        hdr[n] = len as u8;
        out.splice(start..start, hdr[..=n].iter().copied());
    }

    /// A tagged union value: `tag` (never 0) then its record.
    fn tagged(&mut self, out: &mut Vec<u8>, tag: u64, f: impl FnOnce(&mut Self, &mut Vec<u8>)) {
        put_var(out, tag);
        self.record(out, f);
    }

    /// An optional record: `0`, or `1` and the record.
    fn opt_record<T>(
        &mut self,
        out: &mut Vec<u8>,
        v: Option<&'a T>,
        f: impl FnOnce(&mut Self, &mut Vec<u8>, &'a T),
    ) {
        match v {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                self.record(out, |e, o| f(e, o, v));
            }
        }
    }

    fn server_id(&mut self, out: &mut Vec<u8>, id: Option<ServerId>) {
        if self.canonical {
            return;
        }
        let Some(id) = id else {
            out.push(0);
            return;
        };
        match id {
            ServerId::Oid(v) => self.tagged(out, 1, |_, o| put_var(o, u64::from(v))),
            ServerId::ObjectId(v) => self.tagged(out, 2, |_, o| put_i64(o, i64::from(v))),
            ServerId::ObjNum(v) => self.tagged(out, 3, |_, o| put_var(o, v)),
            ServerId::Uuid(v) => self.tagged(out, 4, |_, o| o.extend_from_slice(&v)),
            ServerId::RowId(v) => self.tagged(out, 5, |_, o| put_i64(o, v)),
        }
    }

    fn state(&mut self, out: &mut Vec<u8>, s: StateMark) {
        if self.canonical {
            return;
        }
        match s {
            StateMark::None => out.push(0),
            StateMark::Xmin(v) => self.tagged(out, 1, |_, o| put_var(o, u64::from(v))),
            StateMark::Modified(v) => self.tagged(out, 2, |_, o| put_i64(o, v)),
            StateMark::Created(v) => self.tagged(out, 3, |_, o| put_i64(o, v)),
            StateMark::SchemaVersion(v) => self.tagged(out, 4, |_, o| put_var(o, u64::from(v))),
            StateMark::Hash(v) => self.tagged(out, 5, |_, o| put_var(o, v)),
            StateMark::MyTable {
                created,
                trigger_max,
                trigger_count,
            } => self.tagged(out, 6, |_, o| {
                put_i64(o, created);
                put_i64(o, trigger_max);
                put_var(o, u64::from(trigger_count));
            }),
        }
    }

    fn reference(&mut self, out: &mut Vec<u8>, r: &'a Ref) {
        self.record(out, |e, o| {
            put_var(o, u64::from(r.kind.code()));
            e.names(o, &r.path);
            e.server_id(o, r.server_id);
        });
    }

    fn opt_ref(&mut self, out: &mut Vec<u8>, r: Option<&'a Ref>) {
        match r {
            None => out.push(0),
            Some(r) => {
                out.push(1);
                self.reference(out, r);
            }
        }
    }

    fn watermark(&mut self, out: &mut Vec<u8>, w: &'a Watermark) {
        match w {
            Watermark::None => out.push(0),
            Watermark::PgXid { w, identity } => self.tagged(out, 1, |_, o| {
                put_var(o, *w);
                put_var(o, identity.system_identifier);
                put_i64(o, identity.start_time_micros);
            }),
            Watermark::MyTime {
                at_utc_micros,
                server_uuid,
                version,
                priv_hash,
            } => self.tagged(out, 2, |e, o| {
                put_i64(o, *at_utc_micros);
                e.str(o, server_uuid);
                e.str(o, version);
                put_var(o, *priv_hash);
            }),
            Watermark::Sqlite {
                schema_version,
                file_identity,
                digest,
            } => self.tagged(out, 3, |_, o| {
                put_var(o, u64::from(*schema_version));
                match file_identity {
                    None => o.push(0),
                    Some((dev, ino)) => {
                        o.push(1);
                        put_var(o, *dev);
                        put_var(o, *ino);
                    }
                }
                o.extend_from_slice(&digest.to_le_bytes());
            }),
        }
    }

    fn key_props(&mut self, o: &mut Vec<u8>, p: &'a KeyProps) {
        put_var(o, p.columns.len() as u64);
        for c in &p.columns {
            self.record(o, |e, o| {
                put_var(o, u64::from(c.ordinal));
                e.opt_str(o, c.name.as_ref());
            });
        }
        self.strs(o, &p.expressions);
        self.strs(o, &p.include);
        self.opt_str(o, p.predicate.as_ref());
        put_var(o, p.desc.len() as u64);
        for d in &p.desc {
            put_bool(o, *d);
        }
        put_bool(o, p.unique);
        put_bool(o, p.nulls_not_distinct);
        put_bool(o, p.deferrable);
        put_bool(o, p.initially_deferred);
        put_bool(o, p.not_valid);
        put_bool(o, p.enforced);
        put_bool(o, p.invisible);
        self.opt_str(o, p.backing_index.as_ref());
        self.opt_ref(o, p.implicit_for.as_ref());
        if self.minor >= MINOR_RARE && p.rare.is_some() {
            self.opt_record(o, p.rare.as_deref(), |e, o, r| {
                put_var(o, r.nulls_first.len() as u64);
                for b in &r.nulls_first {
                    put_bool(o, *b);
                }
                e.opt_names(o, &r.opclass);
                e.opt_names(o, &r.collation);
            });
        }
    }

    /// Optional names (per key part): `0`, or `1` and a capped name, each.
    fn opt_names(&mut self, out: &mut Vec<u8>, v: &'a [Option<Name>]) {
        put_var(out, v.len() as u64);
        for s in v {
            match s {
                None => out.push(0),
                Some(s) => {
                    out.push(1);
                    self.name(out, s);
                }
            }
        }
    }

    fn props(&mut self, out: &mut Vec<u8>, p: &'a Props) {
        match p {
            Props::None => out.push(0),
            Props::Database(p) => self.tagged(out, 1, |e, o| {
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.encoding.as_ref());
                e.opt_str(o, p.collation.as_ref());
                e.opt_str(o, p.ctype.as_ref());
                e.opt_str(o, p.comment.as_ref());
            }),
            Props::Schema(p) => self.tagged(out, 2, |e, o| {
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.comment.as_ref());
            }),
            Props::Table(p) => self.tagged(out, 3, |e, o| {
                e.opt_str(o, p.comment.as_ref());
                e.opt_str(o, p.owner.as_ref());
                o.push(match p.persistence {
                    Persistence::Permanent => 0,
                    Persistence::Unlogged => 1,
                    Persistence::Temporary => 2,
                });
                put_bool(o, p.partitioned);
                e.opt_ref(o, p.partition_of.as_ref());
                e.opt_str(o, p.partition_key.as_ref());
                e.opt_str(o, p.partition_bound.as_ref());
                put_bool(o, p.row_security);
                match &p.ext {
                    TableExt::None => o.push(0),
                    TableExt::Pg(x) => e.tagged(o, 1, |e, o| {
                        o.push(x.relkind);
                        e.opt_str(o, x.tablespace.as_ref());
                        e.opt_str(o, x.access_method.as_ref());
                        e.strs(o, &x.options);
                    }),
                    TableExt::MySql {
                        engine,
                        collation,
                        options,
                    } => e.tagged(o, 2, |e, o| {
                        e.opt_str(o, engine.as_ref());
                        e.opt_str(o, collation.as_ref());
                        e.opt_str(o, options.as_ref());
                    }),
                    TableExt::Sqlite {
                        without_rowid,
                        strict,
                        module,
                    } => e.tagged(o, 3, |e, o| {
                        put_bool(o, *without_rowid);
                        put_bool(o, *strict);
                        e.opt_str(o, module.as_ref());
                    }),
                }
            }),
            Props::View(p) => self.tagged(out, 4, |e, o| {
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.comment.as_ref());
                put_source(o, p.source);
                e.opt_str(o, p.check_option.as_ref());
                put_opt_bool(o, p.updatable);
                e.opt_str(o, p.algorithm.as_ref());
                e.opt_str(o, p.definer.as_ref());
                e.opt_str(o, p.sql_security.as_ref());
            }),
            Props::Column(p) => self.tagged(out, 5, |e, o| {
                put_var(o, u64::from(p.ordinal));
                e.str(o, &p.raw_type);
                e.opt_record(o, p.type_ref.as_ref(), |e, o, t| {
                    e.opt_str(o, t.schema.as_ref());
                    e.str(o, &t.name);
                });
                put_bool(o, p.nullable);
                e.opt_str(o, p.default_raw.as_ref());
                e.opt_str(o, p.comment.as_ref());
                put_bool(o, p.hidden);
                e.opt_record(o, p.rare.as_deref(), |e, o, r| {
                    e.opt_str(o, r.not_null_constraint.as_ref());
                    o.push(match r.identity {
                        None => 0,
                        Some(Identity::Always) => 1,
                        Some(Identity::ByDefault) => 2,
                        Some(Identity::AutoIncrement) => 3,
                        Some(Identity::RowidAlias) => 4,
                    });
                    e.opt_record(o, r.generated.as_ref(), |e, o, g| {
                        o.push(match g.kind {
                            GeneratedKind::Stored => 0,
                            GeneratedKind::Virtual => 1,
                        });
                        e.str(o, &g.expr);
                    });
                    e.opt_str(o, r.collation.as_ref());
                    match &r.enum_labels {
                        None => o.push(0),
                        Some(l) => {
                            o.push(1);
                            e.strs(o, l);
                        }
                    }
                });
                // `None` is tag 0; `Some(ColumnExt::None)` is its own tag, so the round trip
                // is exact.
                match p.ext.as_deref() {
                    None => o.push(0),
                    Some(ColumnExt::MySql {
                        column_type,
                        extra,
                        charset,
                    }) => e.tagged(o, 1, |e, o| {
                        e.opt_str(o, column_type.as_ref());
                        e.opt_str(o, extra.as_ref());
                        e.opt_str(o, charset.as_ref());
                    }),
                    Some(ColumnExt::Pg {
                        storage,
                        compression,
                    }) => e.tagged(o, 2, |e, o| {
                        put_opt_u64(o, storage.map(u64::from));
                        e.opt_str(o, compression.as_ref());
                    }),
                    Some(ColumnExt::None) => e.tagged(o, 3, |_, _| {}),
                }
            }),
            Props::Index(p) => self.tagged(out, 6, |e, o| {
                e.record(o, |e, o| e.key_props(o, &p.key));
                e.opt_str(o, p.method.as_ref());
                put_var(o, p.prefix_len.len() as u64);
                for l in &p.prefix_len {
                    put_opt_u64(o, l.map(u64::from));
                }
                e.opt_str(o, p.comment.as_ref());
            }),
            Props::Key(p) => self.tagged(out, 7, |e, o| e.key_props(o, p)),
            Props::ForeignKey(p) => self.tagged(out, 8, |e, o| {
                e.strs(o, &p.columns);
                e.reference(o, &p.target);
                e.strs(o, &p.target_columns);
                o.push(fk_action_code(p.on_update));
                o.push(fk_action_code(p.on_delete));
                o.push(match p.match_kind {
                    FkMatch::Simple => 0,
                    FkMatch::Full => 1,
                    FkMatch::Partial => 2,
                });
                put_bool(o, p.deferrable);
                put_bool(o, p.initially_deferred);
                put_bool(o, p.not_valid);
                put_bool(o, p.enforced);
                if e.minor >= MINOR_RARE && p.rare.is_some() {
                    e.opt_record(o, p.rare.as_deref(), |e, o, r| {
                        e.opt_ref(o, r.fk_ref_index.as_ref());
                    });
                }
            }),
            Props::Check(p) => self.tagged(out, 9, |e, o| {
                e.str(o, &p.expr);
                put_bool(o, p.not_valid);
                put_bool(o, p.enforced);
                if e.minor >= MINOR_RARE && p.rare.is_some() {
                    e.opt_record(o, p.rare.as_deref(), |e, o, r| {
                        put_bool(o, r.no_inherit);
                        e.names(o, &r.excl_ops);
                        put_bool(o, r.excl_deferrable);
                        put_bool(o, r.excl_initially_deferred);
                    });
                }
            }),
            Props::Routine(p) => self.tagged(out, 10, |e, o| {
                match &p.args {
                    None => o.push(0),
                    Some(args) => {
                        o.push(1);
                        put_var(o, args.len() as u64);
                        for a in args {
                            e.record(o, |e, o| {
                                e.opt_str(o, a.name.as_ref());
                                e.str(o, &a.type_name);
                                o.push(match a.mode {
                                    ArgMode::In => 0,
                                    ArgMode::Out => 1,
                                    ArgMode::InOut => 2,
                                    ArgMode::Variadic => 3,
                                    ArgMode::Table => 4,
                                });
                                put_bool(o, a.has_default);
                            });
                        }
                    }
                }
                e.opt_str(o, p.returns.as_ref());
                put_bool(o, p.returns_set);
                o.push(match p.kind {
                    FnKind::Function => 0,
                    FnKind::Aggregate => 1,
                    FnKind::Window => 2,
                    FnKind::Procedure => 3,
                });
                o.push(match p.volatility {
                    Volatility::Unknown => 0,
                    Volatility::Immutable => 1,
                    Volatility::Stable => 2,
                    Volatility::Volatile => 3,
                });
                e.opt_str(o, p.language.as_ref());
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.definer.as_ref());
                e.opt_str(o, p.specific_name.as_ref());
                put_bool(o, p.security_definer);
                e.opt_str(o, p.overload_sig.as_ref());
                put_source(o, p.source);
                e.opt_str(o, p.comment.as_ref());
            }),
            Props::Sequence(p) => self.tagged(out, 11, |e, o| {
                e.opt_str(o, p.owner.as_ref());
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
                        e.reference(o, r);
                        e.str(o, col);
                    }
                }
                e.opt_str(o, p.comment.as_ref());
            }),
            Props::Trigger(p) => self.tagged(out, 12, |e, o| {
                o.push(match p.timing {
                    TriggerTiming::Before => 0,
                    TriggerTiming::After => 1,
                    TriggerTiming::InsteadOf => 2,
                });
                o.push(p.events.bits());
                put_bool(o, p.for_each_row);
                e.opt_ref(o, p.function.as_ref());
                e.opt_str(o, p.when_expr.as_ref());
                put_source(o, p.source);
                put_bool(o, p.enabled);
                e.opt_str(o, p.definer.as_ref());
            }),
            Props::Type(p) => self.tagged(out, 13, |e, o| {
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.comment.as_ref());
                match &p.body {
                    TypeBody::Enum { labels } => e.tagged(o, 1, |e, o| e.strs(o, labels)),
                    TypeBody::Domain {
                        base,
                        not_null,
                        not_null_constraint,
                        default_raw,
                        checks,
                    } => e.tagged(o, 2, |e, o| {
                        e.str(o, base);
                        put_bool(o, *not_null);
                        e.opt_str(o, not_null_constraint.as_ref());
                        e.opt_str(o, default_raw.as_ref());
                        e.strs(o, checks);
                    }),
                    TypeBody::Composite { attrs } => e.tagged(o, 3, |e, o| {
                        put_var(o, attrs.len() as u64);
                        for a in attrs {
                            e.record(o, |e, o| {
                                e.str(o, &a.name);
                                e.str(o, &a.raw_type);
                            });
                        }
                    }),
                    TypeBody::Range { subtype } => e.tagged(o, 4, |e, o| e.str(o, subtype)),
                    TypeBody::Base => o.push(0),
                }
            }),
            Props::Generic(p) => self.tagged(out, 14, |e, o| {
                e.opt_str(o, p.owner.as_ref());
                e.opt_str(o, p.version.as_ref());
                e.opt_str(o, p.comment.as_ref());
                e.opt_str(o, p.raw_kind.as_ref());
            }),
        }
    }

    /// One object record (the parent index is left out in the canonical form, where the
    /// structure is the fingerprint's tree).
    pub(crate) fn obj(&mut self, out: &mut Vec<u8>, o: &'a Obj, pad: &[u8]) {
        self.record(out, |e, b| {
            put_var(b, u64::from(e.code(o.kind())));
            e.name(b, o.name());
            if !e.canonical {
                put_var(
                    b,
                    if o.parent == NO_PARENT {
                        0
                    } else {
                        u64::from(o.parent) + 1
                    },
                );
            }
            put_var(b, u64::from(o.flags().bits()));
            e.server_id(b, o.server_id());
            e.state(b, o.state());
            e.props(b, o.props());
            b.extend_from_slice(pad);
        });
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

fn ns_kind_code(k: NsKind) -> u8 {
    match k {
        NsKind::Server => 0,
        NsKind::Database => 1,
        NsKind::Schema => 2,
    }
}

fn ns_kind(c: u8) -> Option<NsKind> {
    match c {
        0 => Some(NsKind::Server),
        1 => Some(NsKind::Database),
        2 => Some(NsKind::Schema),
        _ => None,
    }
}

fn level_code(l: Level) -> u8 {
    match l {
        Level::Names => 0,
        Level::Structure => 1,
        Level::Sources => 2,
    }
}

fn level(c: u8) -> Level {
    match c {
        1 => Level::Structure,
        2 => Level::Sources,
        _ => Level::Names,
    }
}

fn fold_code(r: FoldRule) -> u8 {
    match r {
        FoldRule::Exact => 0,
        FoldRule::AsciiCaseless => 1,
    }
}

/// The payload of `ns` at the current format version. Deterministic: equal namespaces encode to
/// equal bytes.
pub fn encode_namespace(ns: &NamespaceSnapshot) -> Result<Vec<u8>, DecodeError> {
    encode_as(ns, FORMAT_MINOR, &[])
}

/// Encodes `ns` stamped with `minor`, appending `pad` to every object record and to the body:
/// the stand-in for a newer writer's trailing fields and sections (tests only use other values
/// than `FORMAT_MINOR` and `[]`).
pub(crate) fn encode_as(
    ns: &NamespaceSnapshot,
    minor: u16,
    pad: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    encode_full(ns, minor, pad, None, true)
}

fn encode_full<'a>(
    ns: &'a NamespaceSnapshot,
    minor: u16,
    pad: &[u8],
    remap: Option<(ObjectKind, u16, &'a str)>,
    check_names: bool,
) -> Result<Vec<u8>, DecodeError> {
    if ns.len() > u32::MAX as usize {
        return Err(DecodeError::Malformed("too many objects"));
    }
    let mut e = Enc {
        remap,
        minor,
        ..Enc::table()
    };
    let mut tail = Vec::with_capacity(ns.len() * 24);
    let key = ns.key();
    let meta = ns.meta();
    let mut kinds: Vec<ObjectKind> = ns
        .objs()
        .iter()
        .map(|o| o.kind())
        .filter(|k| *k != ObjectKind::Unknown)
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    e.record(&mut tail, |e, o| {
        o.push(ns_kind_code(key.kind));
        e.names(o, &key.path);
        e.str(o, ns.dbms().id());
        o.push(fold_code(ns.fold()));
        o.push(level_code(meta.level));
        e.watermark(o, &meta.watermark);
        put_i64(o, meta.loaded_at_ms);
        put_var(o, kinds.len() as u64);
        for k in &kinds {
            put_var(o, u64::from(e.code(*k)));
            match e.remap {
                Some((r, _, id)) if r == *k => e.str(o, id),
                _ => e.str(o, k.id()),
            }
        }
        o.extend_from_slice(pad);
    });
    put_var(&mut tail, ns.len() as u64);
    for o in ns.objs() {
        e.obj(&mut tail, o, pad);
    }
    tail.extend_from_slice(pad);
    if e.list.len() > u32::MAX as usize {
        return Err(DecodeError::Malformed("too many strings"));
    }
    if check_names && e.long_name {
        return Err(DecodeError::Malformed("name longer than the cap"));
    }
    let strings_len: usize = e.list.iter().map(|s| s.len() + 2).sum();
    let mut body = Vec::with_capacity(tail.len() + strings_len + 8);
    put_var(&mut body, e.list.len() as u64);
    for s in &e.list {
        put_var(&mut body, s.len() as u64);
        body.extend_from_slice(s.as_bytes());
    }
    body.extend_from_slice(&tail);
    // The decoder's budget, enforced here too, so whatever is written can be read back.
    let budget = body
        .len()
        .saturating_mul(EXPANSION_FACTOR)
        .saturating_add(EXPANSION_SLACK);
    if check_names && e.expanded > budget {
        return Err(DecodeError::Malformed("strings expand past the budget"));
    }

    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    out.extend_from_slice(&minor.to_le_bytes());
    out.push(COMPRESSION_NONE);
    out.extend_from_slice(&twox_hash::XxHash3_64::oneshot(&body).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// The canonical bytes of a namespace key (the cache's key column; associated data of an
/// encrypted row): the kind code, then each path element length-prefixed.
pub fn encode_ns_key(key: &NsKey) -> Vec<u8> {
    let mut out = vec![ns_kind_code(key.kind)];
    for p in &key.path {
        put_var(&mut out, p.len() as u64);
        out.extend_from_slice(p.as_bytes());
    }
    out
}

/// The namespace key in `bytes` ([`encode_ns_key`]'s form).
pub fn decode_ns_key(bytes: &[u8]) -> Result<NsKey, DecodeError> {
    let mut d = Dec::inline(bytes);
    let kind = ns_kind(d.u8()?).ok_or(DecodeError::Malformed("namespace kind"))?;
    let mut path = SmallVec::new();
    while d.more() {
        if path.len() == MAX_NS_PATH {
            return Err(DecodeError::Malformed("namespace path too long"));
        }
        path.push(d.name()?);
    }
    Ok(NsKey { kind, path })
}

/// The standalone bytes of a watermark (the cache's compare-and-set column).
pub fn encode_watermark(w: &Watermark) -> Vec<u8> {
    let mut out = Vec::new();
    Enc::inline().watermark(&mut out, w);
    out
}

/// The watermark in `bytes` ([`encode_watermark`]'s form).
pub fn decode_watermark(bytes: &[u8]) -> Result<Watermark, DecodeError> {
    let mut d = Dec::inline(bytes);
    let w = d.watermark()?;
    d.end()?;
    Ok(w)
}

// ---------------------------------------------------------------------------------------------
// Decoding.

struct Dec<'a> {
    b: &'a [u8],
    pos: usize,
    /// The string table; `None`: strings are inline.
    strings: Option<&'a [Name]>,
    /// The payload is of this build's minor or older: leftover bytes are corruption.
    strict: bool,
    /// String bytes this payload may still expand to (table strings only; B-I3).
    budget: Option<&'a Cell<usize>>,
}

impl<'a> Dec<'a> {
    fn inline(b: &'a [u8]) -> Self {
        Dec {
            b,
            pos: 0,
            strings: None,
            strict: true,
            budget: None,
        }
    }

    fn more(&self) -> bool {
        self.pos < self.b.len()
    }

    fn left(&self) -> usize {
        self.b.len() - self.pos
    }

    /// The end of a record: in strict mode no byte may be left.
    fn end(&self) -> DResult<()> {
        if self.strict && self.more() {
            return Err(DecodeError::Malformed("trailing bytes in a record"));
        }
        Ok(())
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
                    return Err(DecodeError::Malformed("integer out of range"));
                }
                return Ok(v);
            }
        }
        Err(DecodeError::Malformed("integer out of range"))
    }

    fn u32(&mut self) -> DResult<u32> {
        u32::try_from(self.var()?).map_err(|_| DecodeError::Malformed("integer out of range"))
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

    fn array<const N: usize>(&mut self) -> DResult<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.bytes(N)?);
        Ok(a)
    }

    fn len(&mut self) -> DResult<usize> {
        usize::try_from(self.var()?).map_err(|_| DecodeError::Malformed("length out of range"))
    }

    /// A count of elements of at least `min_bytes` bytes each: never more than the bytes left
    /// allow, so a corrupt count cannot trigger a huge allocation.
    fn count(&mut self, min_bytes: usize) -> DResult<usize> {
        let n = self.len()?;
        if n > self.left() / min_bytes.max(1) {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }

    /// A length-prefixed record as its own reader; `f` reads its fields, then the record must
    /// be used up (strict mode).
    fn rec<T>(&mut self, f: impl FnOnce(&mut Dec<'a>) -> DResult<T>) -> DResult<T> {
        let n = self.len()?;
        let mut r = Dec {
            b: self.bytes(n)?,
            pos: 0,
            strings: self.strings,
            strict: self.strict,
            budget: self.budget,
        };
        let v = f(&mut r)?;
        r.end()?;
        Ok(v)
    }

    /// A tagged union value: `default` for tag 0 and for a tag `f` does not know (`f` answers
    /// `None`; the record is skipped).
    fn tagged<T>(
        &mut self,
        default: impl FnOnce() -> T,
        f: impl FnOnce(u64, &mut Dec<'a>) -> DResult<Option<T>>,
    ) -> DResult<T> {
        let tag = self.var()?;
        if tag == 0 {
            return Ok(default());
        }
        let n = self.len()?;
        let mut r = Dec {
            b: self.bytes(n)?,
            pos: 0,
            strings: self.strings,
            strict: self.strict,
            budget: self.budget,
        };
        match f(tag, &mut r)? {
            Some(v) => {
                r.end()?;
                Ok(v)
            }
            None => Ok(default()),
        }
    }

    /// An optional record (`0`, or `1` and the record).
    fn opt_rec<T>(&mut self, f: impl FnOnce(&mut Dec<'a>) -> DResult<T>) -> DResult<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => self.rec(f).map(Some),
            _ => Err(DecodeError::Malformed("optional record marker")),
        }
    }

    fn str(&mut self) -> DResult<Name> {
        match self.strings {
            Some(table) => {
                let i = self.len()?;
                let s = table
                    .get(i)
                    .ok_or(DecodeError::Malformed("string index out of range"))?;
                if let Some(budget) = self.budget {
                    let left = budget
                        .get()
                        .checked_sub(s.len())
                        .ok_or(DecodeError::Malformed("strings expand past the budget"))?;
                    budget.set(left);
                }
                Ok(s.clone())
            }
            None => {
                let n = self.len()?;
                let s = std::str::from_utf8(self.bytes(n)?)
                    .map_err(|_| DecodeError::Malformed("string is not UTF-8"))?;
                Ok(Name::from(s))
            }
        }
    }

    /// A name: a string of at most [`MAX_NAME_BYTES`].
    fn name(&mut self) -> DResult<Name> {
        let s = self.str()?;
        if s.len() > MAX_NAME_BYTES {
            return Err(DecodeError::Malformed("name longer than the cap"));
        }
        Ok(s)
    }

    fn names(&mut self) -> DResult<Vec<Name>> {
        let n = self.count(1)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.name()?);
        }
        Ok(v)
    }

    fn opt_str(&mut self) -> DResult<Option<Name>> {
        match self.u8()? {
            0 => Ok(None),
            1 => self.str().map(Some),
            _ => Err(DecodeError::Malformed("optional string marker")),
        }
    }

    fn strs(&mut self) -> DResult<Vec<Name>> {
        let n = self.count(1)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.str()?);
        }
        Ok(v)
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
            _ => Some(SourceRef {
                hash: u128::from_le_bytes(self.array()?),
                len: self.u32()?,
            }),
        })
    }

    fn server_id(&mut self) -> DResult<Option<ServerId>> {
        self.tagged(
            || None,
            |tag, r| {
                Ok(match tag {
                    1 => Some(Some(ServerId::Oid(r.u32()?))),
                    2 => Some(Some(ServerId::ObjectId(
                        i32::try_from(r.i64()?)
                            .map_err(|_| DecodeError::Malformed("integer out of range"))?,
                    ))),
                    3 => Some(Some(ServerId::ObjNum(r.var()?))),
                    4 => Some(Some(ServerId::Uuid(r.array()?))),
                    5 => Some(Some(ServerId::RowId(r.i64()?))),
                    _ => None,
                })
            },
        )
    }

    fn state(&mut self) -> DResult<StateMark> {
        self.tagged(
            || StateMark::None,
            |tag, r| {
                Ok(match tag {
                    1 => Some(StateMark::Xmin(r.u32()?)),
                    2 => Some(StateMark::Modified(r.i64()?)),
                    3 => Some(StateMark::Created(r.i64()?)),
                    4 => Some(StateMark::SchemaVersion(r.u32()?)),
                    5 => Some(StateMark::Hash(r.var()?)),
                    6 => Some(StateMark::MyTable {
                        created: r.i64()?,
                        trigger_max: r.i64()?,
                        trigger_count: r.u32()?,
                    }),
                    _ => None,
                })
            },
        )
    }

    fn kind(&mut self) -> DResult<(ObjectKind, u16)> {
        let code = u16::try_from(self.var()?).unwrap_or(u16::MAX);
        Ok((
            ObjectKind::from_code(code).unwrap_or(ObjectKind::Unknown),
            code,
        ))
    }

    fn reference(&mut self) -> DResult<Ref> {
        self.rec(|r| {
            Ok(Ref {
                kind: r.kind()?.0,
                path: r.names()?.into_iter().collect(),
                server_id: r.server_id()?,
            })
        })
    }

    fn opt_ref(&mut self) -> DResult<Option<Ref>> {
        Ok(match self.u8()? {
            0 => None,
            1 => Some(self.reference()?),
            _ => return Err(DecodeError::Malformed("optional reference marker")),
        })
    }

    fn watermark(&mut self) -> DResult<Watermark> {
        self.tagged(
            || Watermark::None,
            |tag, r| {
                Ok(match tag {
                    1 => Some(Watermark::PgXid {
                        w: r.var()?,
                        identity: PgIdentity {
                            system_identifier: r.var()?,
                            start_time_micros: r.i64()?,
                        },
                    }),
                    2 => Some(Watermark::MyTime {
                        at_utc_micros: r.i64()?,
                        server_uuid: r.str()?,
                        version: r.str()?,
                        priv_hash: r.var()?,
                    }),
                    3 => Some(Watermark::Sqlite {
                        schema_version: r.u32()?,
                        file_identity: match r.u8()? {
                            0 => None,
                            _ => Some((r.var()?, r.var()?)),
                        },
                        digest: u128::from_le_bytes(r.array()?),
                    }),
                    _ => None,
                })
            },
        )
    }

    fn key_props(&mut self) -> DResult<KeyProps> {
        let n = self.count(2)?;
        let mut columns = Vec::with_capacity(n);
        for _ in 0..n {
            columns.push(self.rec(|c| {
                Ok(KeyColumn {
                    ordinal: c.u32()?,
                    name: c.opt_str()?,
                })
            })?);
        }
        let expressions = self.strs()?;
        let include = self.strs()?;
        let predicate = self.opt_str()?;
        let n = self.count(1)?;
        let mut desc = Vec::with_capacity(n);
        for _ in 0..n {
            desc.push(self.bool()?);
        }
        Ok(KeyProps {
            columns,
            expressions,
            include,
            predicate,
            desc,
            unique: self.bool()?,
            nulls_not_distinct: self.bool()?,
            deferrable: self.bool()?,
            initially_deferred: self.bool()?,
            not_valid: self.bool()?,
            enforced: self.bool()?,
            invisible: self.bool()?,
            backing_index: self.opt_str()?,
            implicit_for: self.opt_ref()?,
            // Minor 1: read only when the record still has bytes (a minor-0 record ends here).
            rare: self.trailing(|r| {
                let n = r.count(1)?;
                let mut nulls_first = Vec::with_capacity(n);
                for _ in 0..n {
                    nulls_first.push(r.bool()?);
                }
                Ok(KeyRare {
                    nulls_first,
                    opclass: r.opt_names()?,
                    collation: r.opt_names()?,
                })
            })?,
        })
    }

    /// A field appended in a later minor at the end of a record: an optional record when the
    /// record still has bytes, `None` when it ends (an older minor's payload).
    fn trailing<T>(
        &mut self,
        f: impl FnOnce(&mut Dec<'a>) -> DResult<T>,
    ) -> DResult<Option<Arc<T>>> {
        if !self.more() {
            return Ok(None);
        }
        Ok(self.opt_rec(f)?.map(Arc::new))
    }

    /// Optional names (per key part), each capped.
    fn opt_names(&mut self) -> DResult<Vec<Option<Name>>> {
        let n = self.count(1)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(match self.u8()? {
                0 => None,
                1 => Some(self.name()?),
                _ => return Err(DecodeError::Malformed("optional name marker")),
            });
        }
        Ok(v)
    }

    fn column(&mut self) -> DResult<ColumnProps> {
        let ordinal = self.u32()?;
        let raw_type = self.str()?;
        let type_ref = self.opt_rec(|t| {
            Ok(TypeRef {
                schema: t.opt_str()?,
                name: t.str()?,
            })
        })?;
        let nullable = self.bool()?;
        let default_raw = self.opt_str()?;
        let comment = self.opt_str()?;
        let hidden = self.bool()?;
        let rare = self.opt_rec(|r| {
            Ok(Arc::new(ColumnRare {
                not_null_constraint: r.opt_str()?,
                identity: match r.u8()? {
                    1 => Some(Identity::Always),
                    2 => Some(Identity::ByDefault),
                    3 => Some(Identity::AutoIncrement),
                    4 => Some(Identity::RowidAlias),
                    _ => None,
                },
                generated: r.opt_rec(|g| {
                    Ok(Generated {
                        kind: match g.u8()? {
                            1 => GeneratedKind::Virtual,
                            _ => GeneratedKind::Stored,
                        },
                        expr: g.str()?,
                    })
                })?,
                collation: r.opt_str()?,
                enum_labels: match r.u8()? {
                    0 => None,
                    _ => Some(Arc::from(r.strs()?)),
                },
            }))
        })?;
        let ext = self.tagged(
            || None,
            |tag, x| {
                Ok(match tag {
                    1 => Some(Some(Arc::new(ColumnExt::MySql {
                        column_type: x.opt_str()?,
                        extra: x.opt_str()?,
                        charset: x.opt_str()?,
                    }))),
                    2 => Some(Some(Arc::new(ColumnExt::Pg {
                        storage: x
                            .opt_u64()?
                            .map(u8::try_from)
                            .transpose()
                            .map_err(|_| DecodeError::Malformed("integer out of range"))?,
                        compression: x.opt_str()?,
                    }))),
                    3 => Some(Some(Arc::new(ColumnExt::None))),
                    _ => None,
                })
            },
        )?;
        Ok(ColumnProps {
            ordinal,
            raw_type,
            type_ref,
            nullable,
            default_raw,
            comment,
            hidden,
            rare,
            ext,
        })
    }

    fn routine(&mut self) -> DResult<RoutineProps> {
        let args = match self.u8()? {
            0 => None,
            _ => {
                let n = self.count(5)?;
                let mut args = Vec::with_capacity(n);
                for _ in 0..n {
                    args.push(self.rec(|a| {
                        Ok(FnArg {
                            name: a.opt_str()?,
                            type_name: a.str()?,
                            mode: match a.u8()? {
                                1 => ArgMode::Out,
                                2 => ArgMode::InOut,
                                3 => ArgMode::Variadic,
                                4 => ArgMode::Table,
                                _ => ArgMode::In,
                            },
                            has_default: a.bool()?,
                        })
                    })?);
                }
                Some(args)
            }
        };
        Ok(RoutineProps {
            args,
            returns: self.opt_str()?,
            returns_set: self.bool()?,
            kind: match self.u8()? {
                1 => FnKind::Aggregate,
                2 => FnKind::Window,
                3 => FnKind::Procedure,
                _ => FnKind::Function,
            },
            volatility: match self.u8()? {
                1 => Volatility::Immutable,
                2 => Volatility::Stable,
                3 => Volatility::Volatile,
                _ => Volatility::Unknown,
            },
            language: self.opt_str()?,
            owner: self.opt_str()?,
            definer: self.opt_str()?,
            specific_name: self.opt_str()?,
            security_definer: self.bool()?,
            overload_sig: self.opt_str()?,
            source: self.source()?,
            comment: self.opt_str()?,
        })
    }

    fn type_body(&mut self) -> DResult<TypeBody> {
        self.tagged(
            || TypeBody::Base,
            |tag, b| {
                Ok(match tag {
                    1 => Some(TypeBody::Enum {
                        labels: Arc::from(b.strs()?),
                    }),
                    2 => Some(TypeBody::Domain {
                        base: b.str()?,
                        not_null: b.bool()?,
                        not_null_constraint: b.opt_str()?,
                        default_raw: b.opt_str()?,
                        checks: b.strs()?,
                    }),
                    3 => {
                        let n = b.count(3)?;
                        let mut attrs = Vec::with_capacity(n);
                        for _ in 0..n {
                            attrs.push(b.rec(|a| {
                                Ok(TypeAttr {
                                    name: a.str()?,
                                    raw_type: a.str()?,
                                })
                            })?);
                        }
                        Some(TypeBody::Composite { attrs })
                    }
                    4 => Some(TypeBody::Range { subtype: b.str()? }),
                    _ => None,
                })
            },
        )
    }

    fn props(&mut self) -> DResult<Props> {
        self.tagged(
            || Props::None,
            |tag, r| {
                Ok(Some(match tag {
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
                        comment: r.opt_str()?,
                        owner: r.opt_str()?,
                        persistence: match r.u8()? {
                            1 => Persistence::Unlogged,
                            2 => Persistence::Temporary,
                            _ => Persistence::Permanent,
                        },
                        partitioned: r.bool()?,
                        partition_of: r.opt_ref()?,
                        partition_key: r.opt_str()?,
                        partition_bound: r.opt_str()?,
                        row_security: r.bool()?,
                        ext: r.tagged(
                            || TableExt::None,
                            |tag, x| {
                                Ok(match tag {
                                    1 => Some(TableExt::Pg(PgTableExt {
                                        relkind: x.u8()?,
                                        tablespace: x.opt_str()?,
                                        access_method: x.opt_str()?,
                                        options: x.strs()?,
                                    })),
                                    2 => Some(TableExt::MySql {
                                        engine: x.opt_str()?,
                                        collation: x.opt_str()?,
                                        options: x.opt_str()?,
                                    }),
                                    3 => Some(TableExt::Sqlite {
                                        without_rowid: x.bool()?,
                                        strict: x.bool()?,
                                        module: x.opt_str()?,
                                    }),
                                    _ => None,
                                })
                            },
                        )?,
                    })),
                    4 => Props::View(Arc::new(ViewProps {
                        owner: r.opt_str()?,
                        comment: r.opt_str()?,
                        source: r.source()?,
                        check_option: r.opt_str()?,
                        updatable: r.opt_bool()?,
                        algorithm: r.opt_str()?,
                        definer: r.opt_str()?,
                        sql_security: r.opt_str()?,
                    })),
                    5 => Props::Column(Arc::new(r.column()?)),
                    6 => {
                        Props::Index(Arc::new(IndexProps {
                            key: r.rec(|k| k.key_props())?,
                            method: r.opt_str()?,
                            prefix_len: {
                                let n = r.count(1)?;
                                let mut v = Vec::with_capacity(n);
                                for _ in 0..n {
                                    v.push(r.opt_u64()?.map(u32::try_from).transpose().map_err(
                                        |_| DecodeError::Malformed("integer out of range"),
                                    )?);
                                }
                                v
                            },
                            comment: r.opt_str()?,
                        }))
                    }
                    7 => Props::Key(Arc::new(r.key_props()?)),
                    8 => Props::ForeignKey(Arc::new(FkProps {
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
                        deferrable: r.bool()?,
                        initially_deferred: r.bool()?,
                        not_valid: r.bool()?,
                        enforced: r.bool()?,
                        rare: r.trailing(|f| {
                            Ok(FkRare {
                                fk_ref_index: f.opt_ref()?,
                            })
                        })?,
                    })),
                    9 => Props::Check(Arc::new(CheckProps {
                        expr: r.str()?,
                        not_valid: r.bool()?,
                        enforced: r.bool()?,
                        rare: r.trailing(|c| {
                            Ok(CheckRare {
                                no_inherit: c.bool()?,
                                excl_ops: c.names()?,
                                excl_deferrable: c.bool()?,
                                excl_initially_deferred: c.bool()?,
                            })
                        })?,
                    })),
                    10 => Props::Routine(Arc::new(r.routine()?)),
                    11 => Props::Sequence(Arc::new(SeqProps {
                        owner: r.opt_str()?,
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
                        definer: r.opt_str()?,
                    })),
                    13 => Props::Type(Arc::new(TypeProps {
                        owner: r.opt_str()?,
                        comment: r.opt_str()?,
                        body: r.type_body()?,
                    })),
                    14 => Props::Generic(Arc::new(GenericProps {
                        owner: r.opt_str()?,
                        version: r.opt_str()?,
                        comment: r.opt_str()?,
                        raw_kind: r.opt_str()?,
                    })),
                    _ => return Ok(None),
                }))
            },
        )
    }

    /// One object. An unknown kind code becomes `Unknown` whose raw kind is the id the
    /// namespace header lists for that code (T3 review m6).
    fn obj(&mut self, kind_ids: &HashMap<u16, Name>) -> DResult<Obj> {
        self.rec(|r| {
            let (kind, code) = r.kind()?;
            let name = r.name()?;
            let parent = match r.var()? {
                0 => NO_PARENT,
                p => u32::try_from(p - 1)
                    .ok()
                    .filter(|p| *p != NO_PARENT)
                    .ok_or(DecodeError::Malformed("parent index out of range"))?,
            };
            let flags = ObjFlags::from_bits_retain(r.u32()?);
            let server_id = r.server_id()?;
            let state = r.state()?;
            let mut props = r.props()?;
            if kind == ObjectKind::Unknown && code != ObjectKind::Unknown.code() {
                props = Props::Generic(Arc::new(GenericProps {
                    comment: props.comment().cloned(),
                    raw_kind: kind_ids.get(&code).cloned(),
                    ..Default::default()
                }));
            }
            let mut o = Obj::new(kind, name)
                .add_flags(flags)
                .with_state(state)
                .with_props(props);
            if let Some(id) = server_id {
                o = o.with_server_id(id);
            }
            o.parent = parent;
            Ok(o)
        })
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

/// Recomputes, in place, the body checksum of an envelope that was assembled or edited outside
/// [`encode_namespace`] (tests and the `catalog_row_decode` fuzz target frame arbitrary bodies
/// with it, so the body decoder sees them). The checksum guards against accidents only; it is
/// not authentication (encrypted rows are authenticated by the row cipher). Shorter than a
/// header: unchanged.
pub fn reseal_checksum(bytes: &mut [u8]) {
    if bytes.len() >= HEADER_LEN {
        let sum = twox_hash::XxHash3_64::oneshot(&bytes[HEADER_LEN..]).to_le_bytes();
        bytes[9..HEADER_LEN].copy_from_slice(&sum);
    }
}

/// The `(major, minor)` of a payload, if its header is readable.
pub fn payload_version(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
        return None;
    }
    Some((
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
    ))
}

/// The namespace in `bytes` (any minor of [`FORMAT_MAJOR`]), its layout validated and its
/// indexes rebuilt.
pub fn decode_namespace(bytes: &[u8]) -> Result<NamespaceSnapshot, DecodeError> {
    decode(bytes).map(|d| d.ns)
}

/// [`decode_namespace`] with the payload's format facts.
pub fn decode(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    decode_as(bytes, FORMAT_MINOR)
}

/// Decodes as a build of format minor `build_minor` would (tests simulate older and newer
/// builds with it).
pub(crate) fn decode_as(bytes: &[u8], build_minor: u16) -> Result<Decoded, DecodeError> {
    decode_with(bytes, build_minor, false)
}

/// [`decode_as`]; with `force_full` the namespace loads with `Watermark::None`, so its next pass
/// is a full introspection (a row marked stale by a server change).
pub(crate) fn decode_with(
    bytes: &[u8],
    build_minor: u16,
    force_full: bool,
) -> Result<Decoded, DecodeError> {
    let (major, minor) = payload_version(bytes).ok_or_else(|| {
        if bytes.len() < HEADER_LEN {
            DecodeError::Truncated
        } else {
            DecodeError::BadMagic
        }
    })?;
    if major != FORMAT_MAJOR {
        return Err(DecodeError::Major { found: major });
    }
    match bytes[8] {
        COMPRESSION_NONE => {}
        c => return Err(DecodeError::Compression(c)),
    }
    let mut sum = [0u8; 8];
    sum.copy_from_slice(&bytes[9..HEADER_LEN]);
    let body = &bytes[HEADER_LEN..];
    if twox_hash::XxHash3_64::oneshot(body) != u64::from_le_bytes(sum) {
        return Err(DecodeError::Checksum);
    }
    let newer = minor > build_minor;
    let strict = !newer;

    let mut d = Dec {
        b: body,
        pos: 0,
        strings: None,
        strict,
        budget: None,
    };
    let n = d.count(1)?;
    let mut strings: Vec<Name> = Vec::with_capacity(n);
    let empty: Name = Name::from("");
    for _ in 0..n {
        let len = d.len()?;
        let s = std::str::from_utf8(d.bytes(len)?)
            .map_err(|_| DecodeError::Malformed("string is not UTF-8"))?;
        strings.push(if s.is_empty() {
            empty.clone()
        } else {
            Name::from(s)
        });
    }
    let budget = Cell::new(
        body.len()
            .saturating_mul(EXPANSION_FACTOR)
            .saturating_add(EXPANSION_SLACK),
    );
    let mut d = Dec {
        b: body,
        pos: d.pos,
        strings: Some(&strings),
        strict,
        budget: Some(&budget),
    };

    let (key, dbms, fold, mut meta, kind_ids) = d.rec(|h| {
        let kind = ns_kind(h.u8()?).ok_or(DecodeError::Malformed("namespace kind"))?;
        let path = h.names()?.into_iter().collect();
        let dbms = DbmsKind::from_id(&h.str()?).ok_or(DecodeError::Malformed("unknown DBMS id"))?;
        let fold = fold_rule(h.u8()?);
        let level = level(h.u8()?);
        let watermark = h.watermark()?;
        let loaded_at_ms = h.i64()?;
        let n = h.count(2)?;
        let mut kind_ids = HashMap::with_capacity(n);
        for _ in 0..n {
            let code = u16::try_from(h.var()?)
                .map_err(|_| DecodeError::Malformed("kind code out of range"))?;
            kind_ids.insert(code, h.str()?);
        }
        Ok((
            NsKey { kind, path },
            dbms,
            fold,
            NsMeta {
                level,
                watermark,
                loaded_at_ms,
            },
            kind_ids,
        ))
    })?;

    let count = d.count(MIN_OBJ_BYTES)?;
    let mut objs = Vec::with_capacity(count);
    for _ in 0..count {
        objs.push(d.obj(&kind_ids)?);
    }
    // A newer minor may append sections after the objects (skipped); ours never does.
    d.end()?;
    if newer || force_full {
        meta.watermark = Watermark::None;
    }
    let mut ns =
        NamespaceSnapshot::from_arena(key, dbms, fold, meta, objs).map_err(DecodeError::Model)?;
    migrate_minor(&mut ns, minor, build_minor);
    Ok(Decoded {
        ns,
        minor,
        from_newer_minor: newer,
    })
}

/// The minor-version migration chain (HV-MODEL-03 step 3): each step is pure and tested.
/// 1.0 → 1.1 (M4 Task 13) needs no step: a 1.0 record has no rare part, which reads as
/// `rare: None`, the value a 1.1 writer stores when nothing is set. A later step reads
/// `if from < 2 && to >= 2 { … }`. A payload from a newer minor is never migrated.
fn migrate_minor(_ns: &mut NamespaceSnapshot, _from: u16, _to: u16) {}

/// Test hooks (feature `test-support`): encode and decode as another format minor would, to
/// exercise the forward- and backward-compatibility rules while this build is at 1.0.
#[cfg(feature = "test-support")]
pub mod testing {
    use super::*;

    /// `ns` encoded as a writer of `minor` would, with `pad` appended to every object record,
    /// to the namespace header and to the body (a newer writer's trailing fields and sections).
    pub fn encode_as_minor(
        ns: &NamespaceSnapshot,
        minor: u16,
        pad: &[u8],
    ) -> Result<Vec<u8>, DecodeError> {
        encode_as(ns, minor, pad)
    }

    /// `bytes` decoded as a build of `build_minor` would.
    pub fn decode_as_build(bytes: &[u8], build_minor: u16) -> Result<Decoded, DecodeError> {
        decode_as(bytes, build_minor)
    }

    /// `ns` encoded with every object of `kind` written under `code` and machine id `id`, as a
    /// newer build would write a kind this build does not know.
    pub fn encode_with_foreign_kind(
        ns: &NamespaceSnapshot,
        kind: ObjectKind,
        code: u16,
        id: &str,
    ) -> Result<Vec<u8>, DecodeError> {
        encode_full(ns, FORMAT_MINOR, &[], Some((kind, code, id)), true)
    }

    /// `ns` encoded without the name cap: a payload the decoder must refuse (the hostile
    /// tests' crafted rows).
    pub fn encode_unchecked(ns: &NamespaceSnapshot) -> Result<Vec<u8>, DecodeError> {
        encode_full(ns, FORMAT_MINOR, &[], None, false)
    }

    pub use super::reseal_checksum;
}

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
                raw_type: Arc::from("integer"),
                ..Default::default()
            }))),
        );
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            DbmsKind::Postgres,
            FoldRule::Exact,
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
            let mut d = Dec::inline(&b);
            assert_eq!(d.var().unwrap(), v);
            assert!(!d.more());
        }
        for v in [0, -1, 1, i64::MIN, i64::MAX] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
        // An eleventh continuation byte, and a tenth byte over 1, are out of range.
        assert!(Dec::inline(&[0xFF; 11]).var().is_err());
        assert!(
            Dec::inline(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02])
                .var()
                .is_err()
        );
    }

    #[test]
    fn an_unknown_union_tag_is_skipped_and_decodes_as_its_default() {
        // Props tag 99 (unknown) with a 2-byte record, then the next field (7).
        let strings: [Name; 0] = [];
        let mut d = Dec {
            b: &[99, 2, 1, 2, 7],
            pos: 0,
            strings: Some(&strings),
            strict: false,
            budget: None,
        };
        assert_eq!(d.props().unwrap(), Props::None);
        assert_eq!(d.u8().unwrap(), 7);
        let mut d = Dec::inline(&[42, 1, 0, 7]);
        assert_eq!(d.watermark().unwrap(), Watermark::None);
        assert_eq!(d.u8().unwrap(), 7);
    }

    #[test]
    fn strict_records_refuse_leftover_bytes() {
        // A server-id record (tag 1, OID 5) with one byte too many.
        let mut d = Dec::inline(&[1, 2, 5, 9]);
        assert!(d.server_id().is_err());
        let mut lenient = Dec {
            strict: false,
            ..Dec::inline(&[1, 2, 5, 9])
        };
        assert_eq!(lenient.server_id().unwrap(), Some(ServerId::Oid(5)));
    }

    #[test]
    fn an_unknown_kind_code_decodes_as_unknown() {
        let mut k = Dec::inline(&[0xE7, 0x07]);
        assert_eq!(k.kind().unwrap(), (ObjectKind::Unknown, 999));
    }

    #[test]
    fn another_major_is_rejected() {
        let mut bytes = encode_namespace(&tiny()).unwrap();
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_namespace(&bytes).unwrap_err(),
            DecodeError::Major { found: 2 }
        );
    }

    #[test]
    fn an_unknown_compression_is_rejected() {
        let mut bytes = encode_namespace(&tiny()).unwrap();
        bytes[8] = 1;
        assert_eq!(
            decode_namespace(&bytes).unwrap_err(),
            DecodeError::Compression(1)
        );
    }

    #[test]
    fn a_flipped_bit_fails_the_checksum() {
        let mut bytes = encode_namespace(&tiny()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x10;
        assert_eq!(decode_namespace(&bytes).unwrap_err(), DecodeError::Checksum);
    }

    #[test]
    fn ns_keys_and_watermarks_round_trip() {
        for key in [
            NsKey::server(),
            NsKey::database("app"),
            NsKey::schema(["app", "İş ılık"]),
            NsKey::schema(["main"]),
        ] {
            assert_eq!(decode_ns_key(&encode_ns_key(&key)).unwrap(), key);
        }
        assert!(decode_ns_key(&[]).is_err());
        assert!(decode_ns_key(&[9]).is_err());
        for w in [
            Watermark::None,
            Watermark::PgXid {
                w: 7,
                identity: PgIdentity {
                    system_identifier: 9,
                    start_time_micros: -3,
                },
            },
            Watermark::MyTime {
                at_utc_micros: 1,
                server_uuid: Name::from("u"),
                version: Name::from("8.4.3"),
                priv_hash: 5,
            },
            Watermark::Sqlite {
                schema_version: 3,
                file_identity: Some((1, 2)),
                digest: u128::MAX - 1,
            },
        ] {
            assert_eq!(decode_watermark(&encode_watermark(&w)).unwrap(), w);
        }
    }
}
