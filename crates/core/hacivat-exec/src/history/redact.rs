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
//! - **Token and key names, SQLCipher keys, secret variables** (## m3b-po-redact, PO M3b-23).
//!   A `TOKEN` part (`token`, `access_token`) is a secret part like `PASSWORD`, and so is a
//!   last part `KEY` after another one (`API_KEY`, `private_key`, `app.api_key`), but not a
//!   table key (`PRIMARY_KEY`, `FOREIGN_KEY`, `UNIQUE_KEY`, …; the bare word `KEY` of
//!   `PRIMARY KEY` or `ON DUPLICATE KEY` arms nothing). In a statement that starts with
//!   `PRAGMA`, the pragmas `key`, `rekey`, `hexkey`, `hexrekey` and `textkey` take a value
//!   (`PRAGMA main.key = '…'`), and in one that starts with `ATTACH`, so does `KEY`. A MySQL
//!   user variable whose name has a secret part or a `PW`, `PASS`, `PASSPHRASE` or `KEY` part
//!   (`@pw`, `@db_pass`, `@key`; never a system variable `@@…`) takes a value after `=` or
//!   `:=` (`SET @pw = '…'`).
//! - **Connection strings.** A quoted string with `password`, `passwd`, `pwd=` or `secret` in
//!   it (ASCII case-insensitive; `token=` and `…_key=` pairs too, m3b-po), or a URI with a
//!   password in its user part
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
//!   (`OPTIONS (apikey '…')`, `SET @v = '…'` then `IDENTIFIED BY @v`), and a value set through
//!   a quoted user variable (`` SET @`pw` = '…' ``).
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
pub(crate) fn scrub(text: &str, secrets: &[String]) -> (String, bool) {
    let r = redact(text);
    let mut out = r.text;
    let mut changed = r.changed;
    for s in secrets.iter().chain(&r.secrets) {
        if !s.is_empty() && out.contains(s.as_str()) {
            out = out.replace(s.as_str(), MASK);
            changed = true;
        }
    }
    (out, changed)
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
    }

    fn mask(&mut self, range: Range<usize>, kind: Kind) {
        self.masks.push(Mask { range, kind });
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
                self.mask(range.clone(), Kind::Meta);
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
                    return;
                }
                if value {
                    self.secret(content);
                }
                let in_call = self.call_string(content);
                let conninfo = looks_like_conninfo(content);
                if conninfo {
                    for v in key_values(content) {
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
                    self.mask(range, Kind::String);
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
            }
            Tok::Ident | Tok::Number => {
                if matches!(self.expect, Expect::Value | Expect::StrictValue) {
                    self.secret(piece);
                    self.mask(range, Kind::Word);
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
        for v in found {
            if v.is_empty() {
                continue;
            }
            let start = inner.0 + (v.as_ptr() as usize - body.as_ptr() as usize);
            self.mask(start..start + v.len(), Kind::Word);
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
        // ## m3b-po-redact (PO M3b-23): the statement's first word.
        if self.head.is_none() {
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
            self.armed = true;
            self.expect = Expect::Value;
            return;
        }
        match self.expect {
            Expect::StrictValue => {
                if !matches!(upper.as_str(), "PASSWORD" | "RANDOM") {
                    self.secret(w);
                    self.mask(range, Kind::Word);
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
                if !is_secret_word(&upper) && !KEYWORDS.contains(&upper.as_str()) {
                    self.secret(w);
                    self.mask(range, Kind::Word);
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
    // ## m3b-po-redact (PO M3b-23): token and `*_KEY` parts too (`app.api_key`).
    let upper = kw_upper(name);
    let parts: Vec<&str> = upper.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    secret_parts(&parts)
}

// ## m3b-po-redact (PO M3b-23)
/// Name parts (ASCII-uppercased) that name a secret: `PASSWORD`, `PASSWD`, `PWD`, `SECRET` or
/// `TOKEN` anywhere (`access_token`), or a last part `KEY` after another part (`API_KEY`,
/// `private_key`) unless that part makes it a table key (`PRIMARY_KEY`, `FOREIGN_KEY`, …).
fn secret_parts(parts: &[&str]) -> bool {
    if parts
        .iter()
        .any(|p| matches!(*p, "PASSWORD" | "PASSWD" | "PWD" | "SECRET" | "TOKEN"))
    {
        return true;
    }
    match parts {
        [.., before, "KEY"] => !matches!(
            *before,
            "" | "PRIMARY"
                | "FOREIGN"
                | "UNIQUE"
                | "SORT"
                | "PARTITION"
                | "DISTRIBUTION"
                | "SHARD"
                | "CLUSTER"
        ),
        _ => false,
    }
}

// ## m3b-po-redact (PO M3b-23)
/// SQLCipher's pragmas that take a key (`upper` from [`kw_upper`]).
const SQLCIPHER_PRAGMAS: &[&str] = &["KEY", "REKEY", "HEXKEY", "HEXREKEY", "TEXTKEY"];

// ## m3b-po-redact (PO M3b-23)
/// A user variable's name (`upper`, without its `@`) naming a secret: a secret word, or a part
/// `PW`, `PASS`, `PASSPHRASE` or `KEY` (`@pw`, `@db_pass`, `@key`).
fn is_secret_variable(upper: &str) -> bool {
    is_secret_word(upper)
        || upper
            .split('_')
            .any(|p| matches!(p, "PW" | "PASS" | "PASSPHRASE" | "KEY"))
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
    // ## m3b-po-redact (PO M3b-23): token and `*_KEY` names too ([`secret_parts`]).
    let parts: Vec<&str> = upper.split('_').collect();
    secret_parts(&parts)
}

/// A string that looks like it carries a credential (a connection string, a URI with a
/// password, a key=value list naming one).
fn looks_like_conninfo(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    // ## m3b-po-redact (PO M3b-23): `token=` and `…_key=` pairs too.
    [
        "password", "passwd", "pwd=", "pwd =", "secret", "token=", "token =", "_key=", "_key =",
    ]
    .iter()
    .any(|k| lower.contains(k))
        || uri_password(s).is_some()
}

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
    // ## m3b-po-redact (PO M3b-23): `token=` and `…_key=` pairs too.
    for key in ["password", "passwd", "pwd", "secret", "token", "_key"] {
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
        let s = redact("CREATE USER u IDENTIFIED BY 'pw4'").secrets;
        let (out, changed) = scrub("near 'BY 'pw4'' at line 1", &s);
        assert!(changed);
        assert!(!out.contains("pw4"), "{out}");
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
    }
}
