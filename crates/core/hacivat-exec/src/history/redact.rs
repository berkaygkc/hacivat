// ## m3b-t01c-core-seams
//! What the history never keeps (Task 17): bind values (masked once in `prepare_units`),
//! secrets in statements (`PASSWORD '…'`, `IDENTIFIED BY …`), error messages past 4 KiB.
// ## m3b-t17-history
//!
//! The writer redacts every statement before it hashes, fingerprints, caps or stores it
//! (`alg/20` HV-EXEC-12 step 2). The scan knows no dialect (a record carries none), so it reads
//! the text twice, once with backslash escapes inside quotes (MySQL) and once without
//! (standard SQL), and masks the union of what either reading finds: a string the two readings
//! disagree about is masked whichever reading is right.
//!
//! - **The two readings' comments.** The MySQL reading takes `#` as a line comment, `--` only
//!   before whitespace, block comments that do not nest, and the bodies of executable comments
//!   (`/*!80000 … */`, `/*M!100000 … */`) as code. The standard reading takes `--`, and block
//!   comments that nest (PostgreSQL). So an apostrophe in `# Bob's` or in a nested
//!   `/* /* */ it's */` cannot hide the statement after it from both readings.
//! - **Secret keywords.** A word `PASSWORD`, `PASSWD`, `PWD`, `SECRET` or `IDENTIFIED`, or a
//!   word with such an `_`-separated part (`MASTER_PASSWORD`, `client_secret`), arms the rest
//!   of its statement (up to a `;`, or a psql meta-command): every quoted string after it
//!   (`'…'`, `"…"`, `$tag$…$tag$`) is masked. The value right after the keyword is masked even
//!   when it is a bare word (Oracle `IDENTIFIED BY tiger`): after `PASSWORD [=|(]`, after
//!   `IDENTIFIED [WITH plugin] BY|AS [PASSWORD|RANDOM]`, after `REPLACE`, and after
//!   `SET PASSWORD FOR <user> =`. A quoted name (`"password" '…'`, `` `passwd` = '…' ``) arms
//!   like the bare word. Keywords are compared through a fixed table, never locale casing:
//!   ASCII capitals, and `İ`/`ı` read as `I` (`İDENTİFİED`, `ıdentıfıed`).
//! - **Token and key names, SQLCipher keys, secret variables** (## m3b-po-redact, PO M3b-23;
//!   narrowed in fix round 1). Secret parts are also `BEARER`, a bare `TOKEN` or a `TOKEN`
//!   after `ACCESS`, `REFRESH`, `API`, `AUTH`, `BEARER`, `SESSION`, `ID`, `CLIENT`, … (never
//!   `token_type`, `token_count`), and a `KEY` after `API`, `ACCESS`, `SECRET`, `PRIVATE`,
//!   `ENCRYPTION`, `MASTER`, `SIGNING`, `AUTH`, `CLIENT`, `SERVICE`, `LICENSE`, `HMAC` or
//!   `SESSION` (`API_KEY`, `aws_access_key_id`; never a surrogate, table or public key such as
//!   `customer_key`, `idempotency_key`, `public_key`, `PRIMARY KEY`). A type name after a
//!   secret column in DDL (`api_key text`) is never its value. After a `PRAGMA` head (or
//!   `EXPLAIN [QUERY PLAN] PRAGMA`), the pragmas `key`, `rekey`, `hexkey`, `hexrekey` and
//!   `textkey` take a value (`PRAGMA main.key = '…'`); after an `ATTACH` head, so does `KEY`.
//!   A MySQL user variable that is a secret name or ends in `PW`, `PASS`, `PASSPHRASE`, `KEY`
//!   or `TOKEN` (`@pw`, `@db_pass`, `@key`; never `@pass_count` or a system variable `@@…`)
//!   takes a value after `=` or `:=`, and `SELECT '…' INTO @pw` masks the statement's strings
//!   before it.
//! - **Connection strings.** A quoted string with `password`, `passwd`, `pwd=` or `secret` in
//!   it (ASCII case-insensitive), or with a `name=value` pair, a JSON pair (`"access_token":
//!   "…"`, `"apiKey":"…"`) or a SQLite URI key (`?key=`, `&hexkey=`, `textkey=`) whose name is
//!   a secret name as above, or with `Bearer <credential>` (m3b-po fix round 1), or a URI with
//!   a password in its user part
//!   (`postgresql://u:pw@h`), is masked wherever it stands (`CREATE SUBSCRIPTION … CONNECTION`,
//!   `dblink_connect(…)`).
//! - **Comments.** A comment's body is read as code by the same rules (`-- password x`), and
//!   the `password=` values and URI passwords in it are masked in place; the rest of the
//!   comment stays.
//! - **Crypto functions and secret settings.** Every string argument of `crypt`,
//!   `pgp_sym_encrypt`/`pgp_sym_decrypt` (and `_bytea`), `pgp_pub_decrypt` (and `_bytea`),
//!   `encrypt`/`decrypt` (and `_iv`), `AES_ENCRYPT`/`AES_DECRYPT` and
//!   `DES_ENCRYPT`/`DES_DECRYPT` is masked; so is `set_config`'s value when its name has a
//!   secret part (`app.secret`).
//! - **psql meta-commands.** From a `\` outside quotes to the end of its line, the text becomes
//!   its `HvNorm1` form (Task 6): `\name` and one `?` for every argument
//!   (`\connect db u password=…` → `\connect ?`).
//!
//! A masked string becomes `'***'`, a masked word `***`. The values found in value positions
//! (and the `password=` values inside connection strings and comments, and the string
//! arguments of the functions above) are the record's *secrets*: the writer also removes them
//! from the error message and the parameter text, since servers quote the failing statement in
//! their errors (MySQL 1064: `near 'BY '…''`). Masking errs towards too much: every string
//! after a secret word in its statement, the next non-keyword word after `PASSWORD`, and a
//! whole string or dollar body that mentions `password` or `secret`.
//!
//! # What redaction does not catch
//!
//! These are known limits (T17 review round 1; m3–m5 are on the PO list; m3's token, `*_KEY`,
//! `PRAGMA key` and `SET @pw` forms are caught since m3b-po). A secret written this way can
//! reach the history file:
//!
//! - **Other secret names** (m3, the rest): a name with none of the parts above
//!   (`OPTIONS (apikey '…')`, `my_token = '…'`, `SET @v = '…'` then `IDENTIFIED BY @v`), and a
//!   value set through a quoted user variable (`` SET @`pw` = '…' ``).
//! - **Short and partial echoes** (fix round 1): the error scrub skips secrets shorter than 4
//!   bytes and replaces only whole occurrences, so a 1–3 byte secret, or one the server glued
//!   to other word characters, stays in the error text.
//! - **URIs** (m4): a password with an unencoded `/`, `#`, `?`, `'` or whitespace ends the
//!   authority early (`postgresql://u:a/b@h`), and a scheme-less `user:password@host` is not
//!   read as a URI. `COPY … PROGRAM 'curl -u u:pw …'` and other shell command lines are kept.
//! - **Error echoes** (m5): the error scrub removes whole secret values only. A server echo cut
//!   short (MySQL's `near '%.80s'` keeps the first 80 characters) keeps the part it shows,
//!   and a string masked only because it follows a secret word, not in value position (the
//!   second of two adjacent strings, `PASSWORD 'a' 'b'`), is masked in the statement but not
//!   removed from the error.
//! - **Bind values** quoted back by the server (`invalid input syntax for type integer:
//!   "…"`): the store never sees the bound value; the run service scrubs it (Task 16).
//! - **Free text and other functions**: a secret in prose (`-- the pw is …`), in a function not
//!   listed above (`hmac(d, '…', 'sha256')`, user-defined functions), or built at run time
//!   (`EXECUTE 'ALTER ROLE r PASSWORD ' || quote_literal(v)`, the value from a column).
//! - **Depth**: code quoted in code, and comments, are read to three levels.

use std::ops::Range;

use sql_dialect::{DialectCtx, DialectId};
use sql_split::hv_norm1;

/// What a masked bare word becomes.
pub(crate) const MASK: &str = "***";
/// What a masked quoted string becomes.
const MASKED_STRING: &str = "'***'";

/// A redacted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Redacted {
    pub text: String,
    /// Something was masked.
    pub changed: bool,
    /// The secret values found (for [`scrub`]).
    pub secrets: Vec<String>,
}

/// How deep the scan reads code quoted inside code for secret values
/// (`DO $$ … EXECUTE 'ALTER ROLE r PASSWORD ''…''' … $$`).
const MAX_DEPTH: u8 = 3;

/// Masks the secrets in `text` (the module docs).
pub(crate) fn redact(text: &str) -> Redacted {
    redact_at(text, 0)
}

/// Both readings of `text` (module docs), their masks and secret values added to `masks` and
/// `secrets` with every range shifted by `offset`.
fn scan_into(
    text: &str,
    depth: u8,
    offset: usize,
    masks: &mut Vec<Mask>,
    secrets: &mut Vec<String>,
) {
    let mut local = Vec::new();
    for backslash in [false, true] {
        Scan::new(text, backslash, depth, &mut local, secrets).run();
    }
    masks.extend(local.into_iter().map(|m| Mask {
        range: m.range.start + offset..m.range.end + offset,
        kind: m.kind,
        role: m.role,
    }));
}

fn redact_at(text: &str, depth: u8) -> Redacted {
    let mut masks = Vec::new();
    let mut secrets = Vec::new();
    scan_into(text, depth, 0, &mut masks, &mut secrets);
    secrets.retain(|s: &String| !s.trim().is_empty() && s != MASK);
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    if masks.is_empty() {
        return Redacted {
            text: text.to_owned(),
            changed: false,
            secrets,
        };
    }
    masks.sort_by_key(|m| (m.range.start, std::cmp::Reverse(m.range.end)));
    let mut merged: Vec<Mask> = Vec::with_capacity(masks.len());
    for m in masks {
        match merged.last_mut() {
            Some(last) if m.range.start < last.range.end => {
                if m.range.end > last.range.end || m.kind != last.kind {
                    last.range.end = last.range.end.max(m.range.end);
                    if m.kind != last.kind {
                        last.kind = Kind::Mixed;
                    }
                }
            }
            _ => merged.push(m),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in &merged {
        out.push_str(&text[at..m.range.start]);
        let piece = &text[m.range.clone()];
        match m.kind {
            Kind::Meta => out.push_str(&hv_norm1(piece, &psql()).normalized),
            Kind::Word => out.push_str(MASK),
            Kind::String => out.push_str(MASKED_STRING),
            Kind::Mixed if piece.starts_with(['\'', '"', '$']) => out.push_str(MASKED_STRING),
            Kind::Mixed => out.push_str(MASK),
        }
        at = m.range.end;
    }
    out.push_str(&text[at..]);
    Redacted {
        text: out,
        changed: true,
        secrets,
    }
}

/// `text` (an error message, the parameter text) redacted, with every one of `secrets`
/// replaced by `***`. Answers whether anything changed.
///
/// ## m3b-po-redact (fix round 1, review I1): only whole occurrences are replaced (a secret
/// that starts or ends with a word character must not continue a longer word there), and
/// only secrets of at least [`MIN_SCRUB`] bytes, so `ERROR 1054 … at line 1` survives a
/// secret `1` and "integer" a secret `int`.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> (String, bool) {
    let r = redact(text);
    let mut out = r.text;
    let mut changed = r.changed;
    for s in secrets.iter().chain(&r.secrets) {
        if s.len() < MIN_SCRUB {
            continue;
        }
        if let Some(next) = replace_whole(&out, s) {
            out = next;
            changed = true;
        }
    }
    (out, changed)
}

// ## m3b-po-redact (fix round 1, I1)
/// The shortest secret [`scrub`] removes from other text.
const MIN_SCRUB: usize = 4;

// ## m3b-po-redact (fix round 1, I1)
/// `out` with every whole occurrence of `s` replaced by [`MASK`]; `None` when there is none.
fn replace_whole(out: &str, s: &str) -> Option<String> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let (b, sb) = (out.as_bytes(), s.as_bytes());
    let (first, last) = (*sb.first()?, *sb.last()?);
    let mut res = String::with_capacity(out.len());
    let (mut at, mut from, mut any) = (0, 0, false);
    while let Some(p) = out[from..].find(s) {
        let i = from + p;
        let j = i + s.len();
        let open = i == 0 || !word(first) || !word(b[i - 1]);
        let close = j == b.len() || !word(last) || !word(b[j]);
        if open && close {
            res.push_str(&out[at..i]);
            res.push_str(MASK);
            at = j;
            from = j;
            any = true;
        } else {
            from = i + s.chars().next().map_or(1, char::len_utf8);
        }
    }
    if !any {
        return None;
    }
    res.push_str(&out[at..]);
    Some(res)
}

fn psql() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Word,
    Meta,
    Mixed,
}

#[derive(Debug, Clone)]
struct Mask {
    range: Range<usize>,
    kind: Kind,
    // ## m4b-t06-secrets (named exception, alg/28 C9): why the range is masked. Only
    // [`secret_spans`] reads it; `redact` and `scrub` ignore it.
    role: Role,
}

// ## m4b-t06-secrets (named exception, alg/28 C9)
/// Why a range is masked: a value position is a secret literal; another string of an armed
/// statement only shares its statement with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The value right after a secret keyword, a crypto function's string argument, a secret
    /// setting's value, or what `INTO @pw` assigns.
    Value,
    /// A `password=` value or a URI password inside a comment.
    Connection,
    /// A whole string masked because it looks like a connection string; its values are found
    /// by [`secret_spans`].
    Conninfo,
    /// A psql meta-command line; its `password=` values are found by [`secret_spans`].
    Meta,
    /// Another string of a statement a secret word armed.
    Armed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// Whitespace, and in the MySQL reading the `/*!80000` / `/*M!` opener and its `*/`.
    Space,
    /// A comment; `inner` is its body.
    Comment {
        inner: (usize, usize),
    },
    /// A quoted string (`'…'`, `"…"`) or a dollar body; `inner` is its content.
    String {
        inner: (usize, usize),
    },
    /// A backquoted identifier.
    Ident,
    Word,
    Number,
    /// `\` outside quotes to the end of the line.
    Meta,
    Semicolon,
    Punct(u8),
}

/// What the scan expects after a secret keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Nothing,
    /// The value after `PASSWORD`-like words: `=` and `(` may come first; a keyword ends it.
    Value,
    /// The value after `IDENTIFIED … BY`, `AS` or `REPLACE`: any word but `PASSWORD`/`RANDOM`.
    StrictValue,
    /// After `IDENTIFIED`: `WITH plugin`, then `BY` or `AS`.
    Identified,
    /// After `IDENTIFIED WITH`: the plugin's name.
    Plugin,
    /// After `PASSWORD FOR`: the user, up to `=`.
    UserThenEq,
}

/// A function whose string arguments carry secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    /// Every string argument is a key or a password (`crypt`, `pgp_sym_encrypt`, …).
    Crypto,
    /// `set_config(name, value, local)`: the value, when the name names a secret.
    SetConfig,
}

#[derive(Debug, Clone, Copy)]
struct Call {
    kind: CallKind,
    /// Parentheses open inside the call (1 = its own argument list).
    depth: u32,
    /// The argument at depth 1 being read.
    arg: u32,
    /// `set_config`'s first argument names a secret.
    secret_key: bool,
}

struct Scan<'a> {
    text: &'a str,
    bytes: &'a [u8],
    backslash: bool,
    depth: u8,
    at: usize,
    masks: &'a mut Vec<Mask>,
    secrets: &'a mut Vec<String>,
    armed: bool,
    identified: bool,
    expect: Expect,
    /// MySQL reading: executable comments (`/*!…`, `/*M!…`) open around the cursor.
    exec: u32,
    /// A secret-carrying function name was read; its `(` comes next.
    pending_call: Option<CallKind>,
    call: Option<Call>,
    // ## m3b-po-redact (PO M3b-23)
    /// The statement's first word (`PRAGMA`, `ATTACH`, …), once read.
    head: Option<Head>,
    // ## m3b-po-redact (fix round 1, m3)
    /// The statement read `INTO` (MySQL `SELECT … INTO @pw`).
    into: bool,
    /// The statement's strings so far, unmasked: a later `INTO @<secret>` masks them.
    strings: Vec<(Range<usize>, (usize, usize))>,
    // ## m4b-t06-secrets (named exception, C9)
    /// The word that set [`Expect::Value`] names the value itself (`PASSWORD`, `api_key`),
    /// not something about it (`password_hash`): [`secret_spans`] reports only the former.
    named: bool,
}

// ## m3b-po-redact (PO M3b-23)
/// What a statement's first word says about the words after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    /// `PRAGMA`: SQLCipher's `key`/`rekey`/`hexkey`/`hexrekey`/`textkey` take a key.
    Pragma,
    /// `ATTACH`: SQLCipher's `KEY '…'` is the attached database's key.
    Attach,
    Other,
}

impl<'a> Scan<'a> {
    fn new(
        text: &'a str,
        backslash: bool,
        depth: u8,
        masks: &'a mut Vec<Mask>,
        secrets: &'a mut Vec<String>,
    ) -> Self {
        Scan {
            text,
            bytes: text.as_bytes(),
            backslash,
            depth,
            at: 0,
            masks,
            secrets,
            armed: false,
            identified: false,
            expect: Expect::Nothing,
            exec: 0,
            pending_call: None,
            call: None,
            head: None,
            into: false,
            strings: Vec::new(),
            named: false,
        }
    }

    fn run(&mut self) {
        while let Some((tok, range)) = self.next() {
            self.step(tok, range);
        }
    }

    fn reset(&mut self) {
        self.armed = false;
        self.identified = false;
        self.expect = Expect::Nothing;
        self.pending_call = None;
        self.call = None;
        self.head = None;
        self.into = false;
        self.strings.clear();
    }

    fn mask(&mut self, range: Range<usize>, kind: Kind, role: Role) {
        self.masks.push(Mask { range, kind, role });
    }

    fn secret(&mut self, s: &str) {
        self.secrets.push(s.to_owned());
    }

    fn step(&mut self, tok: Tok, range: Range<usize>) {
        let piece = &self.text[range.clone()];
        if !matches!(tok, Tok::Space | Tok::Comment { .. }) {
            self.track_call(tok);
        }
        match tok {
            Tok::Space => {}
            Tok::Comment { inner } => self.comment(inner),
            Tok::Semicolon => self.reset(),
            Tok::Meta => {
                self.mask(range.clone(), Kind::Meta, Role::Meta);
                for v in key_values(piece) {
                    self.secret(v);
                }
                // `\set pw …`, `\c "conninfo"`: every argument of three or more characters.
                for arg in piece.split_whitespace().skip(1) {
                    let arg = arg.trim_matches(|c| c == '\'' || c == '"');
                    if arg.len() >= 3 {
                        self.secret(arg);
                    }
                }
                self.reset();
            }
            Tok::String { inner } => {
                let content = &self.text[inner.0..inner.1];
                let value = matches!(self.expect, Expect::Value | Expect::StrictValue);
                // `"password" 'pw'` (a quoted option or column name): the name arms like the
                // bare keyword and stays as it is.
                if !value && self.bytes[range.start] == b'"' && is_secret_word(&kw_upper(content)) {
                    self.armed = true;
                    self.expect = Expect::Value;
                    // ## m4b-t06-secrets (named exception, C9)
                    self.named = names_a_value(&kw_upper(content));
                    return;
                }
                // ## m4b-t06-secrets (named exception, C9)
                let value_role = self.value_role();
                if value {
                    self.secret(content);
                }
                let in_call = self.call_string(content);
                let conninfo = looks_like_conninfo(content);
                if conninfo {
                    for v in key_values(content) {
                        self.secret(v);
                    }
                    // ## m3b-po-redact (fix round 1, I2, m1, m3)
                    for v in string_secrets(content) {
                        self.secret(v);
                    }
                    if let Some(pw) = uri_password(content) {
                        self.secret(pw);
                    }
                    // Code quoted in code: its own secret values, quotes unescaped.
                    if self.depth < MAX_DEPTH {
                        // A dollar body is code as it stands; a quoted string doubles its quote.
                        let unquoted = match self.bytes[range.start] {
                            b'\'' => content.replace("''", "'"),
                            b'"' => content.replace("\"\"", "\""),
                            _ => content.to_owned(),
                        };
                        let inner = redact_at(&unquoted, self.depth + 1);
                        self.secrets.extend(inner.secrets);
                    }
                }
                if self.armed || value || conninfo || in_call {
                    // ## m4b-t06-secrets (named exception, C9)
                    let role = if in_call || (value && value_role == Role::Value) {
                        Role::Value
                    } else if conninfo {
                        Role::Conninfo
                    } else {
                        Role::Armed
                    };
                    self.mask(range, Kind::String, role);
                } else {
                    // ## m3b-po-redact (fix round 1, m3)
                    self.strings.push((range, inner));
                }
                self.after_value();
            }
            Tok::Ident
                if !matches!(self.expect, Expect::Value | Expect::StrictValue)
                    && is_secret_word(&kw_upper(piece.trim_matches('`'))) =>
            {
                // `` `password` = 'pw' ``: as the bare keyword.
                self.armed = true;
                self.expect = Expect::Value;
                // ## m4b-t06-secrets (named exception, C9)
                self.named = names_a_value(&kw_upper(piece.trim_matches('`')));
            }
            Tok::Ident | Tok::Number => {
                if matches!(self.expect, Expect::Value | Expect::StrictValue) {
                    self.secret(piece);
                    let role = self.value_role();
                    self.mask(range, Kind::Word, role);
                }
                self.after_value();
            }
            Tok::Punct(b) => match (self.expect, b) {
                (Expect::Value | Expect::StrictValue, b'=' | b'(') => {}
                // `U&'…'`: the `&` of the Unicode string prefix.
                (Expect::Value | Expect::StrictValue, b'&')
                    if matches!(self.bytes.get(range.end), Some(b'\'' | b'"')) => {}
                // ## m3b-po-redact (PO M3b-23): MySQL's `@pw := '…'`.
                (Expect::Value | Expect::StrictValue, b':')
                    if self.bytes.get(range.end) == Some(&b'=') => {}
                (Expect::UserThenEq, b'=') => self.expect = Expect::Value,
                (Expect::UserThenEq | Expect::Plugin, _) => {}
                _ => self.expect = Expect::Nothing,
            },
            Tok::Word => self.word(piece, range),
        }
    }

    /// Follows the argument list of a secret-carrying call (`crypt(…)`, `set_config(…)`).
    fn track_call(&mut self, tok: Tok) {
        if let Some(kind) = self.pending_call.take()
            && tok == Tok::Punct(b'(')
        {
            self.call = Some(Call {
                kind,
                depth: 0,
                arg: 0,
                secret_key: false,
            });
        }
        let Some(call) = self.call.as_mut() else {
            return;
        };
        match tok {
            Tok::Punct(b'(') => call.depth += 1,
            Tok::Punct(b')') => {
                call.depth = call.depth.saturating_sub(1);
                if call.depth == 0 {
                    self.call = None;
                }
            }
            Tok::Punct(b',') if call.depth == 1 => call.arg += 1,
            _ => {}
        }
    }

    /// A string inside a secret-carrying call: whether it is masked (and then a secret).
    fn call_string(&mut self, content: &str) -> bool {
        let Some(call) = self.call.as_mut() else {
            return false;
        };
        let hide = match call.kind {
            CallKind::Crypto => true,
            CallKind::SetConfig if call.depth == 1 && call.arg == 0 => {
                call.secret_key = names_a_secret(content);
                false
            }
            CallKind::SetConfig => call.depth == 1 && call.arg == 1 && call.secret_key,
        };
        if hide {
            self.secret(content);
        }
        hide
    }

    /// A comment's body is read as code (`-- password x`, `/* … IDENTIFIED BY … */`, the
    /// standard reading of `/*!… */`), and the `password=` values and URI passwords in it are
    /// masked in place. The comment does not change the statement around it.
    fn comment(&mut self, inner: (usize, usize)) {
        let text = self.text;
        let body = &text[inner.0..inner.1];
        if self.depth < MAX_DEPTH {
            scan_into(
                body,
                self.depth + 1,
                inner.0,
                &mut *self.masks,
                &mut *self.secrets,
            );
        }
        let mut found: Vec<&str> = key_values(body);
        found.extend(uri_password(body));
        // ## m3b-po-redact (fix round 1, I2)
        found.extend(string_secrets(body));
        for v in found {
            if v.is_empty() {
                continue;
            }
            let start = inner.0 + (v.as_ptr() as usize - body.as_ptr() as usize);
            self.mask(start..start + v.len(), Kind::Word, Role::Connection);
            self.secret(v);
        }
    }

    /// After a value: `IDENTIFIED BY 'x' REPLACE 'y'` keeps the `IDENTIFIED` chain alive.
    fn after_value(&mut self) {
        self.expect = match self.expect {
            Expect::Plugin => Expect::Identified,
            Expect::UserThenEq => Expect::UserThenEq,
            _ => Expect::Nothing,
        };
    }

    fn word(&mut self, w: &str, range: Range<usize>) {
        // A string prefix (`E'…'`, `N'…'`, `_utf8mb4'…'`): the string is the value.
        let upper = kw_upper(w);
        // ## m3b-po-redact (PO M3b-23): the statement's first word; (fix round 1, m2) after
        // `EXPLAIN [QUERY PLAN]`.
        if self.head.is_none() && !matches!(upper.as_str(), "EXPLAIN" | "QUERY" | "PLAN") {
            self.head = Some(match upper.as_str() {
                "PRAGMA" => Head::Pragma,
                "ATTACH" => Head::Attach,
                _ => Head::Other,
            });
        }
        let next = self.bytes.get(range.end);
        let unicode = upper == "U"
            && next == Some(&b'&')
            && matches!(self.bytes.get(range.end + 1), Some(b'\'' | b'"'));
        if unicode
            || matches!(next, Some(b'\'' | b'"'))
                && (matches!(
                    upper.as_str(),
                    "E" | "N" | "B" | "X" | "U" | "R" | "BR" | "RB"
                ) || upper.starts_with('_'))
        {
            return;
        }
        // `crypt(…)`, `set_config(…)`: their string arguments, not the name, are the secrets.
        if let Some(kind) = call_kind(&upper)
            && matches!(self.expect, Expect::Nothing | Expect::Value)
        {
            self.pending_call = Some(kind);
            self.expect = Expect::Nothing;
            return;
        }
        // ## m3b-po-redact (PO M3b-23): a MySQL user variable named like a secret (`@pw`,
        // `@password`, `@api_key`; never a system variable `@@…`): what is assigned to it is
        // the value, and the rest of the statement is armed.
        let user_var = range.start >= 1
            && self.bytes[range.start - 1] == b'@'
            && (range.start < 2 || self.bytes[range.start - 2] != b'@');
        if user_var && is_secret_variable(&upper) {
            // ## m4b-t06-secrets (named exception, C9): `@pw`, `@db_pass`, `@api_key` name
            // the value; `@password_hash` does not.
            self.named = names_a_value(&upper)
                || matches!(
                    upper.rsplit('_').next(),
                    Some("PW" | "PASS" | "PASSPHRASE" | "KEY" | "TOKEN")
                );
            let role = if self.named { Role::Value } else { Role::Armed };
            // ## m3b-po-redact (fix round 1, m3): `SELECT '…' INTO @pw`: the statement's
            // strings before it are the value.
            if self.into {
                let text = self.text;
                for (r, inner) in std::mem::take(&mut self.strings) {
                    self.secret(&text[inner.0..inner.1]);
                    self.mask(r, Kind::String, role);
                }
            }
            self.armed = true;
            self.expect = Expect::Value;
            return;
        }
        // ## m3b-po-redact (fix round 1, m3)
        if upper == "INTO" {
            self.into = true;
        }
        match self.expect {
            Expect::StrictValue => {
                if !matches!(upper.as_str(), "PASSWORD" | "RANDOM") {
                    self.secret(w);
                    self.mask(range, Kind::Word, Role::Value);
                    self.expect = Expect::Nothing;
                }
                return;
            }
            Expect::Plugin => {
                self.expect = Expect::Identified;
                return;
            }
            Expect::Identified => {
                self.expect = match upper.as_str() {
                    "BY" | "AS" => Expect::StrictValue,
                    "WITH" => Expect::Plugin,
                    _ => Expect::Nothing,
                };
                return;
            }
            Expect::UserThenEq => return,
            Expect::Value => {
                if upper == "FOR" {
                    self.expect = Expect::UserThenEq;
                    return;
                }
                // ## m3b-po-redact (fix round 1, I1): a column's type is no value.
                if !is_secret_word(&upper)
                    && !KEYWORDS.contains(&upper.as_str())
                    && !TYPE_WORDS.contains(&upper.as_str())
                {
                    self.secret(w);
                    let role = self.value_role();
                    self.mask(range, Kind::Word, role);
                    self.expect = Expect::Nothing;
                    return;
                }
                self.expect = Expect::Nothing;
            }
            Expect::Nothing => {}
        }
        if upper == "IDENTIFIED" {
            self.armed = true;
            self.identified = true;
            self.expect = Expect::Identified;
        } else if upper == "REPLACE" && self.identified {
            self.expect = Expect::StrictValue;
        } else if is_secret_word(&upper)
            // ## m3b-po-redact (PO M3b-23): SQLCipher's `PRAGMA [schema.]key = '…'` (and
            // `rekey`, `hexkey`, `hexrekey`, `textkey`) and `ATTACH … KEY '…'`.
            || (self.head == Some(Head::Pragma) && SQLCIPHER_PRAGMAS.contains(&upper.as_str()))
            || (self.head == Some(Head::Attach) && upper == "KEY")
        {
            self.armed = true;
            self.expect = Expect::Value;
            // ## m4b-t06-secrets (named exception, C9): `PASSWORD`, `MASTER_PASSWORD`,
            // `api_key`, `PRAGMA key` name a value; `password_hash`, `passwd_changed` do not.
            self.named = names_a_value(&upper) || !is_secret_word(&upper);
        }
    }

    // ## m4b-t06-secrets (named exception, C9)
    /// The role of a value read now: after `IDENTIFIED … BY` always a value, after a
    /// `PASSWORD`-like word only when the word names the value itself ([`names_a_value`]).
    fn value_role(&self) -> Role {
        if self.expect == Expect::StrictValue || self.named {
            Role::Value
        } else {
            Role::Armed
        }
    }

    /// The next token and its byte range.
    fn next(&mut self) -> Option<(Tok, Range<usize>)> {
        let b = self.bytes;
        let start = self.at;
        let c = *b.get(start)?;
        let tok = match c {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                self.at = self.skip_while(start, |c| c.is_ascii_whitespace());
                Tok::Space
            }
            // `--`: in the MySQL reading only before whitespace or the end (`1--1` is
            // arithmetic there); `#` is a line comment in the MySQL reading only.
            b'-' if b.get(start + 1) == Some(&b'-')
                && (!self.backslash
                    || b.get(start + 2)
                        .is_none_or(|c| c.is_ascii_whitespace() || *c < 0x20)) =>
            {
                self.at = self.line_end(start);
                Tok::Comment {
                    inner: (start + 2, self.at),
                }
            }
            b'#' if self.backslash => {
                self.at = self.line_end(start);
                Tok::Comment {
                    inner: (start + 1, self.at),
                }
            }
            // MySQL reading: `/*!80000 … */` and `/*M!100000 … */` are code; only the opener
            // and the closer are skipped.
            b'/' if self.backslash
                && b.get(start + 1) == Some(&b'*')
                && (b.get(start + 2) == Some(&b'!')
                    || (b.get(start + 2) == Some(&b'M') && b.get(start + 3) == Some(&b'!'))) =>
            {
                let bang = if b[start + 2] == b'!' { 3 } else { 4 };
                self.at = self.skip_while(start + bang, |c| c.is_ascii_digit());
                self.exec += 1;
                Tok::Space
            }
            b'*' if self.backslash && self.exec > 0 && b.get(start + 1) == Some(&b'/') => {
                self.exec -= 1;
                self.at = start + 2;
                Tok::Space
            }
            b'/' if b.get(start + 1) == Some(&b'*') => {
                // MySQL block comments do not nest; standard (PostgreSQL) ones do.
                let (inner_end, end) = if self.backslash {
                    find(b, start + 2, b"*/").map_or((b.len(), b.len()), |e| (e, e + 2))
                } else {
                    self.nested_comment_end(start)
                };
                self.at = end;
                Tok::Comment {
                    inner: (start + 2, inner_end),
                }
            }
            b'\'' | b'"' => {
                let end = self.quoted(start, c);
                self.at = end;
                let close = usize::from(end > start + 1 && b[end - 1] == c && end - 1 > start);
                Tok::String {
                    inner: (start + 1, end - close),
                }
            }
            b'`' => {
                self.at = self.quoted(start, c);
                Tok::Ident
            }
            b'$' => match self.dollar_tag(start) {
                Some(tag_end) => {
                    let tag = &b[start..tag_end];
                    let (inner_end, end) = match find(b, tag_end, tag) {
                        Some(e) => (e, e + tag.len()),
                        None => (b.len(), b.len()),
                    };
                    self.at = end;
                    Tok::String {
                        inner: (tag_end, inner_end),
                    }
                }
                None => {
                    // `$1`: a parameter, read like a number.
                    self.at =
                        self.skip_while(start + 1, |c| c.is_ascii_alphanumeric() || c == b'_');
                    Tok::Number
                }
            },
            b'\\' => {
                self.at = self.line_end(start);
                Tok::Meta
            }
            b';' => {
                self.at = start + 1;
                Tok::Semicolon
            }
            c if c.is_ascii_digit() => {
                self.at = self.skip_while(start, |c| {
                    c.is_ascii_alphanumeric() || c == b'_' || c == b'.'
                });
                Tok::Number
            }
            c if is_word_byte(c) => {
                self.at = self.skip_while(start, |c| {
                    is_word_byte(c) || c.is_ascii_digit() || c == b'$'
                });
                Tok::Word
            }
            c => {
                // One character (a whole UTF-8 sequence for a non-ASCII one: those are words).
                self.at = start + 1;
                Tok::Punct(c)
            }
        };
        Some((tok, start..self.at))
    }

    fn skip_while(&self, mut i: usize, f: impl Fn(u8) -> bool) -> usize {
        while i < self.bytes.len() && f(self.bytes[i]) {
            i += 1;
        }
        i
    }

    /// A nesting block comment opened at `start`: (end of its body, end past its `*/`); the
    /// end of the text for both when it never closes.
    fn nested_comment_end(&self, start: usize) -> (usize, usize) {
        let b = self.bytes;
        let mut depth = 1u32;
        let mut i = start + 2;
        while i + 1 < b.len() {
            match (b[i], b[i + 1]) {
                (b'/', b'*') => {
                    depth += 1;
                    i += 2;
                }
                (b'*', b'/') => {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        return (i - 2, i);
                    }
                }
                _ => i += 1,
            }
        }
        (b.len(), b.len())
    }

    fn line_end(&self, i: usize) -> usize {
        self.skip_while(i, |c| c != b'\n')
    }

    /// The end of the string opened by `q` at `start` (past its closing quote; the end of the
    /// text when it never closes). A doubled quote is one quote; a backslash escapes the next
    /// byte when this reading takes backslash escapes.
    fn quoted(&self, start: usize, q: u8) -> usize {
        let b = self.bytes;
        let mut i = start + 1;
        while i < b.len() {
            match b[i] {
                c if c == q => {
                    if b.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    return i + 1;
                }
                b'\\' if self.backslash && q != b'`' => i += 2,
                _ => i += 1,
            }
        }
        b.len()
    }

    /// `$$` or `$tag$` at `start`: the end of the opening tag.
    fn dollar_tag(&self, start: usize) -> Option<usize> {
        let b = self.bytes;
        let mut i = start + 1;
        if b.get(i).is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
            i += 1;
        }
        (b.get(i) == Some(&b'$')).then_some(i + 1)
    }
}

/// A byte that starts or continues a word: ASCII letters, `_`, and every byte of a non-ASCII
/// character (identifiers may be Turkish).
fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// `w` uppercased for keyword comparison by a fixed table, never locale casing: ASCII letters
/// to their capitals, and the Turkish `İ` and `ı` to `I` (a Turkish keyboard types
/// `İDENTİFİED` with Caps Lock and `ıdentıfıed` without the dot).
fn kw_upper(w: &str) -> String {
    w.chars()
        .map(|c| match c {
            'İ' | 'ı' => 'I',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// Functions whose string arguments are keys or passwords (`upper` from [`kw_upper`]).
const CRYPTO_FUNCTIONS: &[&str] = &[
    "AES_DECRYPT",
    "AES_ENCRYPT",
    "CRYPT",
    "DECRYPT",
    "DECRYPT_IV",
    "DES_DECRYPT",
    "DES_ENCRYPT",
    "ENCRYPT",
    "ENCRYPT_IV",
    "PGP_PUB_DECRYPT",
    "PGP_PUB_DECRYPT_BYTEA",
    "PGP_SYM_DECRYPT",
    "PGP_SYM_DECRYPT_BYTEA",
    "PGP_SYM_ENCRYPT",
    "PGP_SYM_ENCRYPT_BYTEA",
];

fn call_kind(upper: &str) -> Option<CallKind> {
    if CRYPTO_FUNCTIONS.contains(&upper) {
        Some(CallKind::Crypto)
    } else if upper == "SET_CONFIG" {
        Some(CallKind::SetConfig)
    } else {
        None
    }
}

/// A setting name naming a secret: one of its `.`/`_`-separated parts is a secret word
/// (`app.secret`, `myapp.db_password`).
fn names_a_secret(name: &str) -> bool {
    // ## m3b-po-redact (PO M3b-23; fix round 1, I1): token and key names too (`app.api_key`),
    // read in the whole name and in its last dotted segment (`app.token`).
    let upper = kw_upper(name);
    let all: Vec<&str> = upper.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let last = upper.rsplit('.').next().unwrap_or_default();
    let tail: Vec<&str> = last.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    secret_parts(&all) || secret_parts(&tail)
}

// ## m3b-po-redact (fix round 1, I1)
/// The parts before `KEY` that make a key a secret (`API_KEY`, `aws_access_key_id`). Never a
/// table, surrogate or public key (`customer_key`, `public_key`, `idempotency_key`).
const SECRET_KEY_KINDS: &[&str] = &[
    "API",
    "ACCESS",
    "SECRET",
    "PRIVATE",
    "ENCRYPTION",
    "MASTER",
    "SIGNING",
    "AUTH",
    "CLIENT",
    "SERVICE",
    "LICENSE",
    "HMAC",
    "SESSION",
];

// ## m3b-po-redact (fix round 1, I1)
/// The parts before `TOKEN` that make a token a secret (`access_token`, `refresh_token`); a
/// bare `token` is one too. Never `token_type`, `token_count`.
const SECRET_TOKEN_KINDS: &[&str] = &[
    "ACCESS", "REFRESH", "API", "AUTH", "BEARER", "SESSION", "ID", "CLIENT", "SECURITY", "SERVICE",
    "OAUTH", "JWT", "PERSONAL", "SECRET", "PRIVATE", "RESET",
];

// ## m3b-po-redact (PO M3b-23; fix round 1, I1)
/// Name parts (ASCII-uppercased) that name a secret: `PASSWORD`, `PASSWD`, `PWD`, `SECRET` or
/// `BEARER` anywhere; `TOKEN` alone or after a [`SECRET_TOKEN_KINDS`] part; `KEY` after a
/// [`SECRET_KEY_KINDS`] part.
fn secret_parts(parts: &[&str]) -> bool {
    parts.iter().enumerate().any(|(i, p)| {
        let before = i.checked_sub(1).map(|j| parts[j]);
        match *p {
            "PASSWORD" | "PASSWD" | "PWD" | "SECRET" | "BEARER" => true,
            "TOKEN" => parts.len() == 1 || before.is_some_and(|b| SECRET_TOKEN_KINDS.contains(&b)),
            "KEY" => before.is_some_and(|b| SECRET_KEY_KINDS.contains(&b)),
            _ => false,
        }
    })
}

// ## m3b-po-redact (PO M3b-23)
/// SQLCipher's pragmas that take a key (`upper` from [`kw_upper`]).
const SQLCIPHER_PRAGMAS: &[&str] = &["KEY", "REKEY", "HEXKEY", "HEXREKEY", "TEXTKEY"];

// ## m3b-po-redact (PO M3b-23; fix round 1, I1)
/// A user variable's name (`upper`, without its `@`) naming a secret: a secret word, or a last
/// part `PW`, `PASS`, `PASSPHRASE`, `KEY` or `TOKEN` (`@pw`, `@db_pass`, `@key`; never
/// `@pass_count`).
fn is_secret_variable(upper: &str) -> bool {
    is_secret_word(upper)
        || matches!(
            upper.rsplit('_').next(),
            Some("PW" | "PASS" | "PASSPHRASE" | "KEY" | "TOKEN")
        )
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// A word naming a secret: `PASSWORD`, `PASSWD`, `PWD`, `SECRET`, or one with such an
/// `_`-separated part (`upper` is ASCII-uppercased).
fn is_secret_word(upper: &str) -> bool {
    // ## m3b-po-redact (PO M3b-23): token and key names too ([`secret_parts`]).
    let parts: Vec<&str> = upper.split('_').collect();
    secret_parts(&parts)
}

// ## m4b-t06-secrets (named exception, C9)
/// A secret word whose last `_`-separated part is the secret part (`PASSWORD`,
/// `MASTER_PASSWORD`, `client_secret`, `api_key`, `access_token`): what follows it is the value.
/// A word that only mentions one (`password_hash`, `passwd_changed`, `api_key_id`) is not.
fn names_a_value(upper: &str) -> bool {
    let parts: Vec<&str> = upper.split('_').collect();
    let Some((&last, rest)) = parts.split_last() else {
        return false;
    };
    // The last part read as [`secret_parts`] reads it.
    let before = rest.last().copied();
    match last {
        "PASSWORD" | "PASSWD" | "PWD" | "SECRET" | "BEARER" => true,
        "TOKEN" => rest.is_empty() || before.is_some_and(|b| SECRET_TOKEN_KINDS.contains(&b)),
        "KEY" => before.is_some_and(|b| SECRET_KEY_KINDS.contains(&b)),
        _ => false,
    }
}

/// A string that looks like it carries a credential (a connection string, a URI with a
/// password, a key=value list naming one).
fn looks_like_conninfo(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["password", "passwd", "pwd=", "pwd =", "secret"]
        .iter()
        .any(|k| lower.contains(k))
        || uri_password(s).is_some()
        // ## m3b-po-redact (fix round 1, I1, I2, m1, m3)
        || !string_secrets(s).is_empty()
}

// ## m3b-po-redact (fix round 1, I1, I2, m1, m3)
/// The secret values inside a string: `name=value` pairs whose name is a secret name
/// ([`secret_parts`]; `access_token=`, `api_key =`) or a SQLite URI key (`?key=`, `&hexkey=`,
/// `textkey=`), JSON pairs whose name is one (`"access_token": "…"`, `"apiKey":"…"`), and the
/// credential after `Bearer `.
fn string_secrets(s: &str) -> Vec<&str> {
    let mut out = named_pairs(s);
    out.extend(json_pairs(s));
    out.extend(bearer_values(s));
    out
}

/// A name's parts, ASCII-uppercased: split at `_`, `-`, `.` and camelCase humps
/// (`apiKey` → `API`, `KEY`).
fn name_parts(name: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut prev_lower = false;
    for c in name.chars() {
        if matches!(c, '_' | '-' | '.') {
            parts.push(String::new());
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower {
            parts.push(String::new());
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        if let Some(p) = parts.last_mut() {
            p.push(c.to_ascii_uppercase());
        }
    }
    parts
}

fn secret_name(name: &str) -> bool {
    let parts = name_parts(name);
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    secret_parts(&refs)
}

/// The value at `i` in `s`: a quoted run, or a bare run up to white space or a delimiter.
fn value_at<'s>(s: &'s str, mut i: usize, stops: &[char]) -> Option<&'s str> {
    let b = s.as_bytes();
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let (start, end) = match b.get(i) {
        Some(&q @ (b'\'' | b'"')) => {
            let mut e = i + 1;
            while e < b.len() && b[e] != q {
                e += if b[e] == b'\\' { 2 } else { 1 };
            }
            (i + 1, e.min(b.len()))
        }
        _ => {
            let e = s[i..]
                .find(|c: char| c.is_whitespace() || stops.contains(&c))
                .map_or(s.len(), |e| e + i);
            (i, e)
        }
    };
    (end > start && s.is_char_boundary(start) && s.is_char_boundary(end)).then(|| &s[start..end])
}

/// `name=value` pairs with a secret name, or SQLite URI keys after `?`/`&`.
fn named_pairs(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for (eq, _) in s.match_indices('=') {
        let mut e = eq;
        while e > 0 && b[e - 1] == b' ' {
            e -= 1;
        }
        let mut st = e;
        while st > 0 && (b[st - 1].is_ascii_alphanumeric() || b[st - 1] == b'_') {
            st -= 1;
        }
        if st == e || b.get(eq + 1) == Some(&b'=') || (eq > 0 && b[eq - 1] == b'=') {
            continue;
        }
        let name = &s[st..e];
        let sqlite = st > 0
            && matches!(b[st - 1], b'?' | b'&')
            && matches!(
                kw_upper(name).as_str(),
                "KEY" | "HEXKEY" | "TEXTKEY" | "REKEY" | "HEXREKEY"
            );
        if !(sqlite || secret_name(name)) {
            continue;
        }
        out.extend(value_at(s, eq + 1, &['\'', '"', ';', '&', ')', ',']));
    }
    out
}

/// JSON pairs (`"name": "value"`, `"name": value`) with a secret name.
fn json_pairs(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = s[i..].find('"') {
        let start = i + p + 1;
        let Some(q) = s[start..].find('"') else {
            break;
        };
        let end = start + q;
        i = end + 1;
        let name = &s[start..end];
        let mut j = end + 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let named = !name.is_empty()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'));
        if b.get(j) != Some(&b':') || !named || !secret_name(name) {
            continue;
        }
        j += 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if let Some(v) = value_at(s, j, &[',', '}', ']']) {
            out.push(v);
            // Past the value (and its closing quote): always a character boundary.
            i = (v.as_ptr() as usize - s.as_ptr() as usize) + v.len();
            if b.get(i) == Some(&b'"') {
                i += 1;
            }
        }
    }
    out
}

/// The credential after `Bearer ` (`Authorization: Bearer …`).
fn bearer_values(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = lower[from..].find("bearer ") {
        let at = from + p + "bearer ".len();
        from = at;
        out.extend(value_at(s, at, &['\'', '"', ',', ';']));
    }
    out
}

// ## m3b-po-redact (fix round 1, I1)
/// Type names: after a secret column name in DDL (`api_key text`), never its value.
const TYPE_WORDS: &[&str] = &[
    "BIGINT",
    "BIGSERIAL",
    "BINARY",
    "BIT",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BYTEA",
    "CHAR",
    "CHARACTER",
    "CITEXT",
    "CLOB",
    "DATE",
    "DATETIME",
    "DECIMAL",
    "DOUBLE",
    "FLOAT",
    "INET",
    "INT",
    "INT2",
    "INT4",
    "INT8",
    "INTEGER",
    "JSON",
    "JSONB",
    "LONGBLOB",
    "LONGTEXT",
    "MEDIUMBLOB",
    "MEDIUMINT",
    "MEDIUMTEXT",
    "NCHAR",
    "NUMERIC",
    "NVARCHAR",
    "REAL",
    "SERIAL",
    "SMALLINT",
    "TEXT",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYBLOB",
    "TINYINT",
    "TINYTEXT",
    "UUID",
    "VARBINARY",
    "VARCHAR",
];

/// The password of a URI's user part (`scheme://user:PASSWORD@host`).
fn uri_password(s: &str) -> Option<&str> {
    let mut rest = s;
    while let Some(p) = rest.find("://") {
        let auth_start = p + 3;
        let auth = &rest[auth_start..];
        let end = auth
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '\'')
            .unwrap_or(auth.len());
        let authority = &auth[..end];
        if let Some(at) = authority.rfind('@') {
            let user = &authority[..at];
            if let Some(colon) = user.find(':') {
                return Some(&user[colon + 1..]);
            }
        }
        rest = &rest[auth_start..];
    }
    None
}

/// The values of `password=`, `passwd=`, `pwd=` and `secret=` pairs in `s` (ASCII
/// case-insensitive keys, optional spaces around `=`, quoted or bare values).
fn key_values(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    // ## m3b-po-redact (fix round 1, I1): token and key pairs are `named_pairs`' (allow-listed).
    for key in ["password", "passwd", "pwd", "secret"] {
        let mut from = 0;
        while let Some(p) = lower[from..].find(key) {
            let mut i = from + p + key.len();
            from = i;
            let b = s.as_bytes();
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                continue;
            }
            i += 1;
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            let (start, end) = match b.get(i) {
                Some(&q @ (b'\'' | b'"')) => {
                    let e = s[i + 1..].find(q as char).map_or(s.len(), |e| e + i + 1);
                    (i + 1, e)
                }
                _ => {
                    let e = s[i..]
                        .find(|c: char| {
                            c.is_whitespace() || matches!(c, '\'' | '"' | ';' | '&' | ')')
                        })
                        .map_or(s.len(), |e| e + i);
                    (i, e)
                }
            };
            if end > start {
                out.push(&s[start..end]);
            }
        }
    }
    out
}

/// Words that end the wait for a `PASSWORD` value: a column named `password` in a query is not
/// followed by a secret (`SELECT password FROM users`).
const KEYWORDS: &[&str] = &[
    "AND",
    "AS",
    "ASC",
    "BY",
    "CASE",
    "CROSS",
    "CURRENT",
    "DAY",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISCARD",
    "ELSE",
    "ENCRYPTED",
    "END",
    "EXPIRE",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "HISTORY",
    "IN",
    "INNER",
    "INSERT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOCK",
    "NEVER",
    "NOT",
    "NULL",
    "OLD",
    "ON",
    "OPTIONAL",
    "OR",
    "ORDER",
    "OUTER",
    "RANDOM",
    "REQUIRE",
    "RETAIN",
    "REUSE",
    "RIGHT",
    "SELECT",
    "SET",
    "THEN",
    "TO",
    "UNENCRYPTED",
    "UNION",
    "UNLOCK",
    "UPDATE",
    "USING",
    "VALID",
    "VALUES",
    "WHEN",
    "WHERE",
    "WITH",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact(s).text
    }

    #[test]
    fn the_alg_20_case() {
        assert_eq!(
            r("ALTER ROLE app PASSWORD 'x'"),
            "ALTER ROLE app PASSWORD '***'"
        );
    }

    #[test]
    fn bare_values_are_masked() {
        assert_eq!(
            r("CREATE USER u IDENTIFIED BY tiger"),
            "CREATE USER u IDENTIFIED BY ***"
        );
        assert_eq!(
            r("ALTER USER u IDENTIFIED BY tiger REPLACE lion"),
            "ALTER USER u IDENTIFIED BY *** REPLACE ***"
        );
        assert_eq!(
            r("CREATE USER u IDENTIFIED BY RANDOM PASSWORD"),
            "CREATE USER u IDENTIFIED BY RANDOM PASSWORD"
        );
    }

    #[test]
    fn a_password_column_in_a_query_is_left_alone() {
        let q = "SELECT password FROM users WHERE id = 1";
        assert_eq!(r(q), q);
        assert!(!redact(q).changed);
    }

    #[test]
    fn psql_meta_commands_keep_only_their_name() {
        assert_eq!(r("\\connect db u password=x"), "\\connect ?");
        assert_eq!(
            r("SELECT 1;\n\\c host=h password=x\nSELECT 2"),
            "SELECT 1;\n\\c ?\nSELECT 2"
        );
        assert_eq!(r("\\password"), "\\password");
    }

    #[test]
    fn secrets_are_the_values() {
        let s = redact("SET PASSWORD FOR 'u'@'h' = 'pw1'").secrets;
        assert_eq!(s, ["pw1"]);
        let s = redact("SELECT dblink_connect('host=h password=pw2 user=u')").secrets;
        assert!(s.contains(&"pw2".to_owned()));
        let s = redact("SELECT f('postgresql://u:pw3@h/db')").secrets;
        assert!(s.contains(&"pw3".to_owned()));
    }

    // ## m3b-po-redact (PO M3b-23): token and `*_KEY` names, SQLCipher keys, secret user
    // variables. The secrets here are fixture values, asserted absent.
    #[test]
    fn token_and_key_names_sqlcipher_keys_and_user_variables_are_masked() {
        for (sql, want, secret) in [
            (
                "CREATE SERVER s FOREIGN DATA WRAPPER w OPTIONS (token 'hv_tk1')",
                "CREATE SERVER s FOREIGN DATA WRAPPER w OPTIONS (token '***')",
                "hv_tk1",
            ),
            (
                "SET app.api_token = 'hv_tk2'",
                "SET app.api_token = '***'",
                "hv_tk2",
            ),
            (
                "UPDATE cfg SET access_token = hv_tk3 WHERE id = 1",
                "UPDATE cfg SET access_token = *** WHERE id = 1",
                "hv_tk3",
            ),
            (
                "ALTER SERVER s OPTIONS (ADD api_key 'hv_k1')",
                "ALTER SERVER s OPTIONS (ADD api_key '***')",
                "hv_k1",
            ),
            (
                "UPDATE t SET PRIVATE_KEY = 'hv_k2'",
                "UPDATE t SET PRIVATE_KEY = '***'",
                "hv_k2",
            ),
            (
                "SELECT set_config('app.api_key', 'hv_k3', false)",
                "SELECT set_config('app.api_key', '***', false)",
                "hv_k3",
            ),
            (
                "SELECT dblink_connect('host=h access_token=hv_k4')",
                "SELECT dblink_connect('***')",
                "hv_k4",
            ),
            ("PRAGMA key = 'hv_k5'", "PRAGMA key = '***'", "hv_k5"),
            ("pragma KEY='hv_k6';", "pragma KEY='***';", "hv_k6"),
            ("PRAGMA rekey = \"hv_k7\"", "PRAGMA rekey = '***'", "hv_k7"),
            (
                "PRAGMA main.hexkey = \"x'2DD29CA851E7'\"",
                "PRAGMA main.hexkey = '***'",
                "2DD29CA851E7",
            ),
            ("PRAGMA key = x'0A1B2C'", "PRAGMA key = x'***'", "0A1B2C"),
            (
                "ATTACH DATABASE 'x.db' AS x KEY 'hv_k8'",
                "ATTACH DATABASE 'x.db' AS x KEY '***'",
                "hv_k8",
            ),
            ("SET @pw = 'hv_pw1'", "SET @pw = '***'", "hv_pw1"),
            ("SET @pw := 'hv_pw2'", "SET @pw := '***'", "hv_pw2"),
            (
                "SELECT @password := 'hv_pw3'",
                "SELECT @password := '***'",
                "hv_pw3",
            ),
            ("SET @db_pass = hv_pw4", "SET @db_pass = ***", "hv_pw4"),
            ("SET @Key = 'hv_pw5'", "SET @Key = '***'", "hv_pw5"),
            (
                "SET @tok = 0, @token = 'hv_pw6'",
                "SET @tok = 0, @token = '***'",
                "hv_pw6",
            ),
        ] {
            let r = redact(sql);
            assert_eq!(r.text, want, "{sql}");
            assert!(r.changed, "{sql}");
            assert!(!r.text.contains(secret), "{sql}");
            assert!(r.secrets.iter().any(|s| s.contains(secret)), "{sql}");
        }
        // The variable's value is a secret the error scrub removes.
        let s = redact("SET @pw = 'hv_pw7'; CREATE USER u IDENTIFIED BY @pw").secrets;
        let (out, _) = scrub("near 'hv_pw7' at line 1", &s);
        assert!(!out.contains("hv_pw7"), "{out}");
    }

    #[test]
    fn keys_tokens_and_variables_that_hold_no_secret_are_kept() {
        for q in [
            "SET FOREIGN_KEY_CHECKS = 0",
            "SET unique_key_checks = 1",
            "ALTER TABLE t ADD PRIMARY KEY (id)",
            "ALTER TABLE t ADD CONSTRAINT fk FOREIGN KEY (a) REFERENCES u (id)",
            "CREATE TABLE t (a int, KEY idx (a), b varchar(9) DEFAULT 'x')",
            "INSERT INTO t VALUES ('a') ON DUPLICATE KEY UPDATE b = 'x'",
            "SELECT primary_key, foreign_key FROM t WHERE x = 'y'",
            "SELECT * FROM keys WHERE k = 'v'",
            "PRAGMA foreign_keys = ON",
            "PRAGMA journal_mode = 'wal'",
            "PRAGMA table_info('t')",
            "ATTACH DATABASE 'x.db' AS x",
            "SET @n = 5",
            "SET @name = 'bob'",
            "SELECT @@max_connections, @@session.sql_mode",
            "SELECT 'token', 'api key'",
            "UPDATE t SET monkey = 'x', turnkey = 'y'",
        ] {
            let r = redact(q);
            assert_eq!(r.text, q);
            assert!(!r.changed, "{q}");
        }
    }

    #[test]
    fn scrub_removes_secrets_from_other_text() {
        // ## m3b-po-redact (fix round 1, I1): a scrubbed secret is at least 4 bytes.
        let s = redact("CREATE USER u IDENTIFIED BY 'pw4x'").secrets;
        let (out, changed) = scrub("near 'BY 'pw4x'' at line 1", &s);
        assert!(changed);
        assert!(!out.contains("pw4x"), "{out}");
    }

    // ## m3b-po-redact (fix round 1, review I1, I2, m1-m3)
    /// A masked value must be gone from the text and be one of the record's secrets.
    fn masks(sql: &str, secret: &str) {
        let r = redact(sql);
        assert!(r.changed, "{sql}");
        assert!(!r.text.contains(secret), "{sql} → {}", r.text);
        assert!(
            r.secrets.iter().any(|s| s.contains(secret)),
            "{sql}: secret recorded"
        );
    }

    #[test]
    fn json_pairs_sqlite_uri_keys_explain_pragma_into_and_bearer_are_masked() {
        for (sql, secret) in [
            // I2: JSON pairs with a secret name.
            (
                "INSERT INTO cfg VALUES ('{\"access_token\": \"ghp_Hv1qz\"}')",
                "ghp_Hv1qz",
            ),
            ("UPDATE integ SET cfg = '{\"api_key\":\"Hv2qz\"}'", "Hv2qz"),
            ("SELECT '{\"token\":\"Hv3qz\"}'::jsonb", "Hv3qz"),
            (
                "SELECT '{\"a\": 1, \"refresh_token\" : \"Hv4qz\"}'",
                "Hv4qz",
            ),
            ("SELECT '{\"client_secret\":\"Hv5qz\",\"x\":2}'", "Hv5qz"),
            ("SELECT '{\"private_key\": \"Hv6qz\"}'", "Hv6qz"),
            ("SELECT '{\"apiKey\": \"Hv7qz\"}'", "Hv7qz"),
            ("SELECT '{\"accessToken\":\"Hv8qz\"}'", "Hv8qz"),
            ("SELECT '{\"password\": \"Hv9qz\"}'", "Hv9qz"),
            // m1: SQLite URI keys.
            ("ATTACH DATABASE 'file:x.db?key=Hv10qz' AS x", "Hv10qz"),
            (
                "ATTACH DATABASE 'file:x.db?mode=ro&hexkey=Hv11qz' AS x",
                "Hv11qz",
            ),
            ("ATTACH 'file:x.db?textkey=Hv12qz' AS x", "Hv12qz"),
            // m2: EXPLAIN in front of PRAGMA.
            ("EXPLAIN PRAGMA key = 'Hv13qz'", "Hv13qz"),
            ("EXPLAIN QUERY PLAN PRAGMA rekey = 'Hv14qz'", "Hv14qz"),
            // m3: SELECT … INTO a secret variable, bearer names and headers.
            ("SELECT 'Hv15qz' INTO @pw", "Hv15qz"),
            ("SELECT 'a', 'Hv16qz' INTO @x, @api_key", "Hv16qz"),
            (
                "SELECT http_get('https://h', 'Authorization: Bearer Hv17qz')",
                "Hv17qz",
            ),
            ("UPDATE t SET bearer = 'Hv18qz'", "Hv18qz"),
            ("UPDATE t SET bearer_token = 'Hv19qz'", "Hv19qz"),
            // The allow-listed key and token names.
            ("UPDATE t SET aws_access_key_id = 'Hv20qz'", "Hv20qz"),
            ("UPDATE t SET encryption_key = 'Hv21qz'", "Hv21qz"),
            ("UPDATE t SET signing_key = 'Hv22qz'", "Hv22qz"),
            ("UPDATE t SET refresh_token = 'Hv23qz'", "Hv23qz"),
            ("UPDATE t SET auth_token = 'Hv24qz'", "Hv24qz"),
            (
                "SELECT f('https://h/x?access_token=Hv25qz&page=2')",
                "Hv25qz",
            ),
            ("SELECT set_config('app.token', 'Hv26qz', false)", "Hv26qz"),
        ] {
            masks(sql, secret);
        }
        // A column's type is never its value; the default after it is.
        assert_eq!(
            r("CREATE TABLE t (api_key text DEFAULT 'Hv27qz')"),
            "CREATE TABLE t (api_key text DEFAULT '***')"
        );
    }

    #[test]
    fn surrogate_keys_token_counts_and_column_types_are_kept() {
        for q in [
            "SELECT * FROM fact_sales WHERE customer_key = 42 AND region = 'EU'",
            "UPDATE dim_date SET date_key = 20240101 WHERE full_date = '2024-01-01'",
            "UPDATE t SET idempotency_key = 'abc-123'",
            "UPDATE t SET cache_key = 'user:42'",
            "UPDATE t SET object_key = 'a/b.png', row_key = 'r1'",
            "UPDATE t SET public_key = 'ssh-ed25519 AAAA'",
            "UPDATE t SET sort_key = 1, partition_key = 'p'",
            "UPDATE t SET token_type = 'refresh', token_count = 3",
            "SET @pass_count := 3",
            "SET @token_count = 3",
            "SELECT f('https://h/x?sort_key=name&page=2')",
            "SELECT '{\"token_type\": \"refresh\", \"count\": 2}'",
            "SELECT 'file:x.db?mode=ro'",
            "SELECT 'x' INTO @n",
            "CREATE TABLE t (customer_key int PRIMARY KEY, name text DEFAULT 'x')",
            "CREATE TABLE users (password varchar(64) NOT NULL)",
            "ALTER TABLE t ADD COLUMN api_key text",
            "EXPLAIN QUERY PLAN SELECT * FROM t WHERE k = 'v'",
        ] {
            let r = redact(q);
            assert_eq!(r.text, q);
            assert!(!r.changed, "{q}");
            assert!(r.secrets.is_empty(), "{q}: {:?}", r.secrets.len());
        }
    }

    #[test]
    fn scrub_replaces_whole_secrets_of_four_bytes_or_more_only() {
        let err =
            "ERROR 1054 (42S22) at line 1: Unknown column 'x' in 'field list'; integer syntax";
        let short: Vec<String> = ["1", "int", "x", "054"].map(String::from).to_vec();
        assert_eq!(scrub(err, &short), (err.to_owned(), false));
        let s = vec!["Hv77qz".to_owned()];
        let (out, changed) = scrub("near 'Hv77qz' and xHv77qzy at line 1", &s);
        assert!(changed);
        assert_eq!(out, "near '***' and xHv77qzy at line 1");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]
        /// Byte offsets never cut a character, whatever the text.
        #[test]
        fn redaction_never_panics(s in "\\PC{0,80}") {
            let _ = scrub(&s, &redact(&s).secrets);
        }

        /// SQL-shaped text with quotes, escapes, dollars, meta-commands and Turkish letters.
        #[test]
        fn sql_shaped_text_never_panics(
            s in "(PASSWORD|IDENTIFIED|BY|WITH|FOR|REPLACE|U&|E|\\$\\$|\\$t\\$|'|\"|`|\\\\|;|=|\\(|\\)|/\\*|\\*/|--|\n| |ş|İ|x|1|password=|://u:p@h){0,40}"
        ) {
            let r = redact(&s);
            let _ = scrub(&s, &r.secrets);
        }

        // ## m3b-po-redact (fix round 1): the pair, JSON, bearer and INTO readers.
        #[test]
        fn pair_and_json_text_never_panics(
            s in "('|\"|\\{|\\}|:|=|\\?|&|,|;| |\u{a0}|ş|İ|x|1|\\\\|token|api_key|accessToken|key|hexkey|Bearer |INTO|@pw|PRAGMA|EXPLAIN|file:){0,40}"
        ) {
            let r = redact(&s);
            let _ = scrub(&s, &r.secrets);
        }
    }
}

// ## m4b-t01a-seams
/// What a [`secret_spans`] range is (alg/28 C9): the redactor masks every quoted string of an
/// armed statement, but only a value position is a secret literal for the script workspace's
/// save warning (`WHERE password_hash = 'x'` must not warn). `hacivat_scripts::SpanKind`
/// mirrors it (the app maps `Value` → `Password`, `ConnectionValue` → `ConnectionString`,
/// `Armed` → `Other`; R-M4b-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretSpanKind {
    /// The value right after a secret keyword (`PASSWORD 'x'`, `IDENTIFIED BY tiger`).
    Value,
    /// A connection-string or URI password value (`password=…`, `scheme://user:pass@host`).
    ConnectionValue,
    /// Another string the redactor masks because a secret word armed its statement.
    Armed,
}

/// The byte ranges of `text` the history redactor would mask, each with its kind (the script
/// workspace's secret scanner takes them from the app, R-M4b-28).
///
/// ## m4b-t06-secrets (named exception: the body)
/// Built from `scan_into`'s masks, the very scan `redact` uses, so every range `redact` masks
/// lies inside a span (`secret_spans_cover_what_redact_masks`). A value position (alg/28 C9)
/// is `Value`; a comment's `password=` value or URI password is `ConnectionValue`; every other
/// mask is `Armed`. Inside an `Armed` connection string or psql meta-command line, its
/// `password=` values, secret `name=value` and JSON pairs, `Bearer` credentials and URI
/// password come back as `ConnectionValue` spans, and code quoted in code (a dollar body, a
/// string with doubled quotes) is read again to [`MAX_DEPTH`] for its quoted values. Sorted by
/// start, without duplicates; every range lies on character boundaries of `text`.
pub fn secret_spans(text: &str) -> Vec<(Range<usize>, SecretSpanKind)> {
    let mut out = Vec::new();
    spans_into(text, 0, &|r| r, &mut out);
    out.sort_by_key(|(r, k)| (r.start, std::cmp::Reverse(r.end), *k as u8));
    out.dedup();
    out
}
// ## m4b-t01a-seams

// ## m4b-t06-secrets
/// `secret_spans` of `text` read at `depth`, each range mapped into the outer text by `map`
/// (a range of `text` to a range of the outermost text).
fn spans_into(
    text: &str,
    depth: u8,
    map: &dyn Fn(Range<usize>) -> Range<usize>,
    out: &mut Vec<(Range<usize>, SecretSpanKind)>,
) {
    let mut masks = Vec::new();
    let mut secrets = Vec::new();
    scan_into(text, depth, 0, &mut masks, &mut secrets);
    for m in masks {
        let nested = depth > 0;
        let piece = &text[m.range.clone()];
        let kind = match m.role {
            // Read again, code's bare words are too often names (`EXECUTE 'GRANT … ' ||
            // password_col`): only quoted values count there.
            Role::Value if nested && !piece.starts_with(['\'', '"', '$']) => SecretSpanKind::Armed,
            Role::Value => SecretSpanKind::Value,
            Role::Connection => SecretSpanKind::ConnectionValue,
            Role::Conninfo | Role::Meta | Role::Armed => SecretSpanKind::Armed,
        };
        out.push((map(m.range.clone()), kind));
        if !matches!(m.role, Role::Conninfo | Role::Meta) {
            continue;
        }
        let mut values: Vec<&str> = key_values(piece);
        values.extend(string_secrets(piece));
        values.extend(uri_password(piece));
        for v in values {
            if v.is_empty() {
                continue;
            }
            let start = m.range.start + (v.as_ptr() as usize - piece.as_ptr() as usize);
            out.push((map(start..start + v.len()), SecretSpanKind::ConnectionValue));
        }
        if m.role == Role::Conninfo && depth < MAX_DEPTH {
            quoted_code(piece, m.range.start, depth + 1, map, out);
        }
    }
}

// ## m4b-t06-secrets
/// The spans of the code quoted in `piece` (a masked string token starting at `at` of the text
/// `map` maps from): a dollar body as it stands, or a `'…'`/`"…"` string with a doubled quote
/// (`EXECUTE 'ALTER ROLE r PASSWORD ''x'''`), read with each doubled quote as one.
fn quoted_code(
    piece: &str,
    at: usize,
    depth: u8,
    map: &dyn Fn(Range<usize>) -> Range<usize>,
    out: &mut Vec<(Range<usize>, SecretSpanKind)>,
) {
    let b = piece.as_bytes();
    match b.first() {
        Some(b'$') => {
            // `$tag$ … $tag$`: the body between the tags (an unclosed body runs to the end).
            let Some(tag_len) = piece[1..].find('$').map(|p| p + 2) else {
                return;
            };
            let tag = &piece[..tag_len];
            let body_end = if piece.len() >= 2 * tag_len && piece.ends_with(tag) {
                piece.len() - tag_len
            } else {
                piece.len()
            };
            let body = &piece[tag_len..body_end];
            let base = at + tag_len;
            spans_into(body, depth, &|r| map(r.start + base..r.end + base), out);
        }
        Some(&q @ (b'\'' | b'"')) => {
            let end = if b.len() >= 2 && b[b.len() - 1] == q {
                b.len() - 1
            } else {
                b.len()
            };
            let cb = &b[1..end];
            if !cb.windows(2).any(|w| w == [q, q]) {
                return;
            }
            // `starts[i]`/`ends[i]`: where byte `i` of the unescaped text starts and ends in
            // `piece`; a doubled quote is two bytes of `piece` for one (ASCII, so the
            // unescaped bytes stay UTF-8).
            let mut bytes = Vec::with_capacity(cb.len());
            let mut starts = Vec::with_capacity(cb.len() + 1);
            let mut ends = Vec::with_capacity(cb.len());
            let mut i = 0;
            while i < cb.len() {
                let width = if cb[i] == q && cb.get(i + 1) == Some(&q) {
                    2
                } else {
                    1
                };
                bytes.push(cb[i]);
                starts.push(1 + i);
                ends.push(1 + i + width);
                i += width;
            }
            starts.push(1 + cb.len());
            let Ok(unescaped) = String::from_utf8(bytes) else {
                return;
            };
            spans_into(
                &unescaped,
                depth,
                &|r: Range<usize>| {
                    let start = at + starts[r.start];
                    let end = if r.end > r.start {
                        at + ends[r.end - 1]
                    } else {
                        start
                    };
                    map(start..end)
                },
                out,
            );
        }
        _ => {}
    }
}
// ## m4b-t06-secrets

// ## m4b-t06-secrets
#[cfg(test)]
mod m4b_t06_tests {
    use super::*;
    use proptest::prelude::*;

    /// Every range `redact` masks lies inside one `secret_spans` range.
    fn covered(text: &str) -> Result<(), String> {
        let mut masks = Vec::new();
        let mut secrets = Vec::new();
        scan_into(text, 0, 0, &mut masks, &mut secrets);
        let spans = secret_spans(text);
        for m in &masks {
            if !spans
                .iter()
                .any(|(r, _)| r.start <= m.range.start && m.range.end <= r.end)
            {
                return Err(format!("{text:?}: {:?} not in {spans:?}", m.range));
            }
        }
        Ok(())
    }

    #[test]
    fn value_and_armed_are_told_apart() {
        // C9: `password_hash = 'x'` is masked in history but is no value position.
        let t = "SELECT * FROM users WHERE password_hash = 'x'";
        assert_eq!(secret_spans(t), [(42..45, SecretSpanKind::Armed)]);
        let t = "ALTER ROLE app PASSWORD 'x'";
        assert_eq!(secret_spans(t), [(24..27, SecretSpanKind::Value)]);
        let t = "CREATE USER u IDENTIFIED BY tiger";
        assert_eq!(secret_spans(t), [(28..33, SecretSpanKind::Value)]);
        // A connection string: the whole string is masked; its password is the value.
        let t = "SELECT dblink_connect('host=h password=pw')";
        assert_eq!(
            secret_spans(t),
            [
                (22..42, SecretSpanKind::Armed),
                (39..41, SecretSpanKind::ConnectionValue)
            ]
        );
        assert!(secret_spans("SELECT 1").is_empty());
        // The word names the value (`MASTER_PASSWORD`, `@pw`) or only mentions one.
        let value = |t: &str| {
            secret_spans(t)
                .iter()
                .any(|(_, k)| *k == SecretSpanKind::Value)
        };
        for t in [
            "UPDATE users SET password = 's3cret'",
            "CHANGE MASTER TO MASTER_PASSWORD='s3cret'",
            "SELECT 's3cret' INTO @pw",
            "SET @db_pass = 's3cret'",
            "UPDATE t SET api_key = 's3cret'",
        ] {
            assert!(value(t), "{t}");
        }
        for t in [
            "SET @password_hash = 's3cret'",
            "SELECT 'x' INTO @password_hash",
            "UPDATE t SET api_key_id = 's3cret'",
            "UPDATE t SET passwd_changed = '2030-01-01'",
        ] {
            assert!(!value(t), "{t}");
            assert!(!secret_spans(t).is_empty(), "{t}: still masked in history");
        }
    }

    #[test]
    fn spans_cover_every_mask_of_the_corpus() {
        for t in [
            "ALTER ROLE app PASSWORD 's3cret'",
            "\\connect db u password=s3cret",
            "SELECT 'a\\'; ALTER ROLE r PASSWORD 's3cret'",
            "DO $$ BEGIN EXECUTE 'ALTER ROLE r PASSWORD ''s3cret'''; END $$",
            "-- password s3cret\nSELECT 1",
            "SELECT '' INTO @pw",
        ] {
            covered(t).unwrap();
        }
    }

    proptest! {
        #[test]
        fn spans_cover_every_mask(
            s in "(PASSWORD|IDENTIFIED|BY|WITH|FOR|REPLACE|U&|E|\\$\\$|\\$t\\$|'|''|\"|`|\\\\|;|=|\\(|\\)|/\\*|\\*/|--|\n| |ş|İ|x|1|password=|://u:p@h|pgp_sym_encrypt|INTO|@pw|PRAGMA|key){0,40}"
        ) {
            prop_assert!(covered(&s).is_ok(), "{}", covered(&s).unwrap_err());
            for (r, _) in secret_spans(&s) {
                prop_assert!(s.get(r.clone()).is_some(), "{s:?}: {r:?}");
            }
        }
    }
}
// ## m4b-t06-secrets
