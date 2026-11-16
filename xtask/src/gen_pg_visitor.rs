//! `cargo xtask gen-pg-visitor [--check]` (M3b Task 9; R-M3-2): writes
//! `crates/sql/sql-safety/src/visit/pg_generated.rs`, the exhaustive visitor over libpg_query's
//! parse tree, from the vendored `third_party/pg_query/libpg_query/protobuf/pg_query.proto`.
//!
//! No `protoc`: a small proto3 reader below takes exactly what the file uses (top-level
//! `message` and `enum` blocks, fields `[repeated] <type> <name> = <n> [options];`, `oneof`
//! blocks, and messages nested in `SummaryResult` with one `map<…>` field). Anything else is an
//! error, so a schema change the reader does not understand stops the generator instead of
//! producing a visitor that silently skips a field.
//!
//! The generated file holds:
//! - `MsgKind`: every top-level message, with `name`, `from_name` and `ALL`;
//! - `PgRef<'a>`: a borrowed message of any kind, with `kind`;
//! - `NODE_VARIANTS` and `node_inner`/`node_variant`: the `Node` oneof, variant by variant;
//! - `push_children`: every message-typed field of every message (singular, repeated and
//!   `oneof` arms), pushed in reverse field order so a stack pops them in field order;
//! - `walk`: an iterative pre-order walk with an explicit stack (no recursion, so tree depth
//!   never reaches the thread's stack) that calls a `Visitor`'s `enter` and `leave`.
//!
//! The names follow prost's conversion (`heck`): message and oneof-variant names in
//! UpperCamelCase (`SQLValueFunction` → `SqlValueFunction`, `sqlvalue_function` →
//! `SqlvalueFunction`), modules in snake_case (`A_Const` → `a_const`), and Rust keywords as raw
//! identifiers. A wrong name does not compile, so the conversion is checked by the build.
//!
//! The output is piped through `rustfmt` (as `gen-sql-tables` does) and starts with
//! `// proto-digest: xxh3-64:<hex>` of the schema (CRLF read as LF), which `sql-safety`'s
//! `generated_visitor_is_fresh` test compares with the vendored file. `--check` regenerates
//! in memory and fails when the checked-in file differs (CI, `# ## m3b-t09-visitor`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, anyhow, bail, ensure};
use twox_hash::XxHash3_64;

/// The schema, relative to the workspace root.
pub const PROTO: &str = "third_party/pg_query/libpg_query/protobuf/pg_query.proto";
/// The generated visitor, relative to the workspace root.
pub const OUT: &str = "crates/sql/sql-safety/src/visit/pg_generated.rs";

/// Messages that are not part of a parse tree (only `pg_query::summary` produces them); their
/// nested messages and `map` fields are allowed and not walked.
const OUTSIDE_THE_TREE: [&str; 1] = ["SummaryResult"];

/// One field of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    name: String,
    ty: String,
    repeated: bool,
}

/// A `oneof` block: its name and its arms.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Oneof {
    name: String,
    arms: Vec<Field>,
}

/// A member of a message, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Member {
    Field(Field),
    Oneof(Oneof),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    name: String,
    members: Vec<Member>,
}

/// The schema: top-level messages in file order, and every enum name.
#[derive(Debug, Default)]
struct Schema {
    messages: Vec<Message>,
    enums: BTreeSet<String>,
}

/// Proto scalar types (leaves).
const SCALARS: [&str; 15] = [
    "double", "float", "int32", "int64", "uint32", "uint64", "sint32", "sint64", "fixed32",
    "fixed64", "sfixed32", "sfixed64", "bool", "string", "bytes",
];

/// `text` without `//` and `/* */` comments (no string literal in the schema holds either).
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |e| &rest[l + e..]);
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                rest = rest[b + 2..]
                    .find("*/")
                    .map_or("", |e| &rest[b + 2 + e + 2..]);
                out.push(' ');
            }
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// The schema's tokens: words, numbers, and the punctuation `{ } [ ] = ; < > , "…"`.
fn tokens(text: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if d.is_ascii_alphanumeric() || d == '_' || d == '.' {
                    end = j + d.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(text[i..end].to_string());
        } else if c == '"' {
            let mut s = String::from("\"");
            loop {
                let (_, d) = chars.next().ok_or_else(|| anyhow!("unterminated string"))?;
                s.push(d);
                if d == '"' {
                    break;
                }
            }
            out.push(s);
        } else if "{}[]=;<>,".contains(c) {
            out.push(c.to_string());
        } else {
            bail!("unexpected character {c:?} in the schema");
        }
    }
    Ok(out)
}

/// A cursor over the tokens.
struct Toks {
    toks: Vec<String>,
    at: usize,
}

impl Toks {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.at).map(String::as_str)
    }

    fn next(&mut self) -> Result<String> {
        let t = self
            .toks
            .get(self.at)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected end of the schema"))?;
        self.at += 1;
        Ok(t)
    }

    fn expect(&mut self, want: &str) -> Result<()> {
        let got = self.next()?;
        ensure!(got == want, "expected {want:?}, found {got:?}");
        Ok(())
    }

    /// Skips `[ … ]` field options.
    fn options(&mut self) -> Result<()> {
        if self.peek() == Some("[") {
            while self.next()? != "]" {}
        }
        Ok(())
    }
}

/// Reads `pg_query.proto`.
fn parse_schema(text: &str) -> Result<Schema> {
    let mut t = Toks {
        toks: tokens(&strip_comments(text))?,
        at: 0,
    };
    let mut schema = Schema::default();
    while let Some(word) = t.peek() {
        match word {
            "syntax" => {
                t.next()?;
                t.expect("=")?;
                ensure!(t.next()? == "\"proto3\"", "only proto3 is supported");
                t.expect(";")?;
            }
            "package" => {
                t.next()?;
                t.next()?;
                t.expect(";")?;
            }
            "enum" => {
                t.next()?;
                schema.enums.insert(t.next()?);
                skip_block(&mut t)?;
            }
            "message" => {
                t.next()?;
                let name = t.next()?;
                let message = parse_message(&mut t, name)?;
                schema.messages.push(message);
            }
            other => bail!("unexpected {other:?} at the top level of the schema"),
        }
    }
    Ok(schema)
}

/// Skips a `{ … }` block, nested blocks included.
fn skip_block(t: &mut Toks) -> Result<()> {
    t.expect("{")?;
    let mut depth = 1;
    while depth > 0 {
        match t.next()?.as_str() {
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}

/// Reads one field after its optional `repeated`: `<type> <name> = <n> [options];`.
fn parse_field(t: &mut Toks, repeated: bool) -> Result<Field> {
    let ty = t.next()?;
    let name = t.next()?;
    t.expect("=")?;
    let tag = t.next()?;
    ensure!(
        tag.bytes().all(|b| b.is_ascii_digit()),
        "field {name}: bad tag {tag:?}"
    );
    t.options()?;
    t.expect(";")?;
    Ok(Field { name, ty, repeated })
}

fn parse_message(t: &mut Toks, name: String) -> Result<Message> {
    t.expect("{")?;
    let mut members = Vec::new();
    let outside = OUTSIDE_THE_TREE.contains(&name.as_str());
    loop {
        match t.peek() {
            Some("}") => {
                t.next()?;
                return Ok(Message { name, members });
            }
            Some("oneof") => {
                t.next()?;
                let oneof = t.next()?;
                t.expect("{")?;
                let mut arms = Vec::new();
                while t.peek() != Some("}") {
                    arms.push(parse_field(t, false)?);
                }
                t.expect("}")?;
                members.push(Member::Oneof(Oneof { name: oneof, arms }));
            }
            Some("repeated") => {
                t.next()?;
                members.push(Member::Field(parse_field(t, true)?));
            }
            Some("message" | "enum") if outside => {
                t.next()?;
                t.next()?;
                skip_block(t)?;
            }
            Some("map") if outside => while t.next()? != ";" {},
            Some(_) => members.push(Member::Field(parse_field(t, false)?)),
            None => bail!("message {name} does not close"),
        }
    }
}

/// `name` split into words as `heck` splits it: at `_`, at a lower-to-upper case change, and
/// before the last capital of a run of capitals that a lower-case letter follows
/// (`SQLValue` → `SQL`, `Value`). Digits stay in the current word.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in name.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<char> = part.chars().collect();
        let mut word = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1).copied();
            let boundary = c.is_ascii_uppercase()
                && prev.is_some_and(|p| {
                    p.is_ascii_lowercase()
                        || p.is_ascii_digit()
                        || (p.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase()))
                });
            if boundary && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}

/// prost's UpperCamelCase (`heck::ToUpperCamelCase`).
fn upper_camel(name: &str) -> String {
    words(name)
        .iter()
        .map(|w| {
            let mut cs = w.chars();
            let first = cs.next().map(|c| c.to_ascii_uppercase());
            first
                .into_iter()
                .chain(cs.map(|c| c.to_ascii_lowercase()))
                .collect::<String>()
        })
        .collect()
}

/// prost's snake_case (`heck::ToSnakeCase`).
fn snake(name: &str) -> String {
    words(name)
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// Rust keywords prost escapes as raw identifiers.
const KEYWORDS: [&str; 51] = [
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

/// A field's Rust name.
fn field_ident(name: &str) -> String {
    let s = snake(name);
    if KEYWORDS.contains(&s.as_str()) {
        format!("r#{s}")
    } else {
        s
    }
}

/// The generated visitor for `proto` (the schema's text), formatted.
fn generate(proto: &str) -> Result<String> {
    let schema = parse_schema(proto)?;
    let names: BTreeMap<&str, String> = schema
        .messages
        .iter()
        .map(|m| (m.name.as_str(), upper_camel(&m.name)))
        .collect();
    ensure!(
        names.values().collect::<BTreeSet<_>>().len() == names.len(),
        "two messages map to one Rust name"
    );
    let is_message = |ty: &str| names.contains_key(ty);
    // Every field type is a message, an enum or a scalar: nothing is skipped by accident.
    for m in &schema.messages {
        if OUTSIDE_THE_TREE.contains(&m.name.as_str()) {
            continue;
        }
        for member in &m.members {
            let fields: Vec<&Field> = match member {
                Member::Field(f) => vec![f],
                Member::Oneof(o) => o.arms.iter().collect(),
            };
            for f in fields {
                ensure!(
                    is_message(&f.ty)
                        || schema.enums.contains(&f.ty)
                        || SCALARS.contains(&f.ty.as_str()),
                    "{}.{}: unknown type {}",
                    m.name,
                    f.name,
                    f.ty
                );
            }
        }
    }
    let node = schema
        .messages
        .iter()
        .find(|m| m.name == "Node")
        .ok_or_else(|| anyhow!("no Node message"))?;
    let node_arms = match node.members.as_slice() {
        [Member::Oneof(o)] if o.name == "node" => &o.arms,
        _ => bail!("Node is not one `oneof node`"),
    };

    let digest = format!(
        "xxh3-64:{:016x}",
        XxHash3_64::oneshot(proto.replace("\r\n", "\n").as_bytes())
    );
    let mut o = String::new();
    writeln!(o, "// proto-digest: {digest}")?;
    o.push_str(HEADER);

    // MsgKind
    o.push_str(
        "/// Every top-level message of the schema.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\n\
         pub(crate) enum MsgKind {\n",
    );
    for m in &schema.messages {
        writeln!(o, "    {},", names[m.name.as_str()])?;
    }
    o.push_str("}\n\nimpl MsgKind {\n");
    writeln!(
        o,
        "    /// Every kind, in schema order.\n    pub(crate) const ALL: [MsgKind; {}] = [",
        schema.messages.len()
    )?;
    for m in &schema.messages {
        writeln!(o, "        MsgKind::{},", names[m.name.as_str()])?;
    }
    o.push_str("    ];\n\n");
    o.push_str(
        "    /// The message's name in the schema.\n    pub(crate) fn name(self) -> &'static str {\n        match self {\n",
    );
    for m in &schema.messages {
        writeln!(
            o,
            "            MsgKind::{} => \"{}\",",
            names[m.name.as_str()],
            m.name
        )?;
    }
    o.push_str("        }\n    }\n\n");
    o.push_str(
        "    /// The kind named `name` in the schema.\n    pub(crate) fn from_name(name: &str) -> Option<MsgKind> {\n        Some(match name {\n",
    );
    for m in &schema.messages {
        writeln!(
            o,
            "            \"{}\" => MsgKind::{},",
            m.name,
            names[m.name.as_str()]
        )?;
    }
    o.push_str("            _ => return None,\n        })\n    }\n}\n\n");

    // PgRef
    o.push_str(
        "/// A borrowed message of any kind.\n#[derive(Debug, Clone, Copy)]\n\
         #[allow(dead_code, reason = \"visitors read the messages they have rules for\")]\n\
         pub(crate) enum PgRef<'a> {\n",
    );
    for m in &schema.messages {
        let n = &names[m.name.as_str()];
        writeln!(o, "    {n}(&'a pb::{n}),")?;
    }
    o.push_str("}\n\nimpl PgRef<'_> {\n    /// The message's kind.\n    pub(crate) fn kind(self) -> MsgKind {\n        match self {\n");
    for m in &schema.messages {
        let n = &names[m.name.as_str()];
        writeln!(o, "            PgRef::{n}(_) => MsgKind::{n},")?;
    }
    o.push_str("        }\n    }\n}\n\n");

    // Node variants
    writeln!(
        o,
        "/// The arms of `Node`'s `oneof node`: prost's variant name and the message it holds.\n\
         pub(crate) const NODE_VARIANTS: [(&str, MsgKind); {}] = [",
        node_arms.len()
    )?;
    for a in node_arms {
        ensure!(is_message(&a.ty), "Node.{} holds no message", a.name);
        writeln!(
            o,
            "    (\"{}\", MsgKind::{}),",
            upper_camel(&a.name),
            names[a.ty.as_str()]
        )?;
    }
    o.push_str("];\n\n");
    o.push_str(
        "/// The message a `Node` holds; `None` for an empty `Node`.\n\
         pub(crate) fn node_inner(n: &pb::Node) -> Option<PgRef<'_>> {\n    use pb::node::Node as N;\n    Some(match n.node.as_ref()? {\n",
    );
    for a in node_arms {
        writeln!(
            o,
            "        N::{}(m) => PgRef::{}(m),",
            upper_camel(&a.name),
            names[a.ty.as_str()]
        )?;
    }
    o.push_str("    })\n}\n\n");
    o.push_str(
        "/// prost's variant name of the message a `Node` holds; `None` for an empty `Node`.\n\
         pub(crate) fn node_variant(n: &pb::Node) -> Option<&'static str> {\n    use pb::node::Node as N;\n    Some(match n.node.as_ref()? {\n",
    );
    for a in node_arms {
        let v = upper_camel(&a.name);
        writeln!(o, "        N::{v}(_) => \"{v}\",")?;
    }
    o.push_str("    })\n}\n\n");

    // push_children
    o.push_str(
        "/// Pushes every message-typed field of `r` onto `out` (singular fields that are set,\n\
         /// every element of repeated fields, the arm of a `oneof` that holds a message), in\n\
         /// reverse field order, so a stack pops them in field order.\n\
         #[allow(clippy::too_many_lines, reason = \"one arm per message of the schema\")]\n\
         pub(crate) fn push_children<'a>(r: PgRef<'a>, out: &mut Vec<PgRef<'a>>) {\n    match r {\n",
    );
    for m in &schema.messages {
        let n = &names[m.name.as_str()];
        let outside = OUTSIDE_THE_TREE.contains(&m.name.as_str());
        let mut body = String::new();
        if !outside {
            for member in m.members.iter().rev() {
                match member {
                    Member::Field(f) if is_message(&f.ty) => {
                        let id = field_ident(&f.name);
                        let t = &names[f.ty.as_str()];
                        if f.repeated {
                            writeln!(
                                body,
                                "            for c in m.{id}.iter().rev() {{\n                out.push(PgRef::{t}(c));\n            }}"
                            )?;
                        } else {
                            writeln!(
                                body,
                                "            if let Some(c) = &m.{id} {{\n                out.push(PgRef::{t}(c));\n            }}"
                            )?;
                        }
                    }
                    Member::Field(_) => {}
                    Member::Oneof(oneof) => {
                        let module = snake(&m.name);
                        let en = upper_camel(&oneof.name);
                        let id = field_ident(&oneof.name);
                        if m.name == "Node" {
                            writeln!(
                                body,
                                "            if let Some(c) = node_inner(m) {{\n                out.push(c);\n            }}"
                            )?;
                            continue;
                        }
                        writeln!(
                            body,
                            "            if let Some(v) = &m.{id} {{\n                match v {{"
                        )?;
                        for a in &oneof.arms {
                            let v = upper_camel(&a.name);
                            if is_message(&a.ty) {
                                writeln!(
                                    body,
                                    "                    pb::{module}::{en}::{v}(c) => out.push(PgRef::{}(c)),",
                                    names[a.ty.as_str()]
                                )?;
                            } else {
                                writeln!(
                                    body,
                                    "                    pb::{module}::{en}::{v}(_) => {{}}"
                                )?;
                            }
                        }
                        writeln!(body, "                }}\n            }}")?;
                    }
                }
            }
        }
        if body.is_empty() {
            writeln!(o, "        PgRef::{n}(_) => {{}}")?;
        } else {
            writeln!(o, "        PgRef::{n}(m) => {{\n{body}        }}")?;
        }
    }
    o.push_str("    }\n}\n");
    o.push_str(WALK);
    rustfmt(&o)
}

/// The module docs and imports of the generated file.
const HEADER: &str = "//! Generated by `cargo xtask gen-pg-visitor` from
//! `third_party/pg_query/libpg_query/protobuf/pg_query.proto` (M3b Task 9; R-M3-2). Do not edit:
//! change the generator and run it again. CI fails when this file is stale, and
//! `generated_visitor_is_fresh` compares the digest on the first line with the schema.
//!
//! An exhaustive visitor over libpg_query's parse tree: [`push_children`] names every
//! message-typed field of every message, and [`walk`] visits every message reachable from its
//! root with an explicit stack (no recursion). The classifier (`backend::pg`) decides what each
//! message means; it never walks the tree by hand.

use sql_oracle_pg::tree::pb;

";

/// The walk, the same for every schema.
const WALK: &str = "
/// What a visitor wants after [`Visitor::enter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Walk {
    /// Visit the message's children, then call [`Visitor::leave`].
    Children,
    /// Skip the children (the visitor walks them itself if it needs to); no `leave` call.
    #[allow(dead_code, reason = \"no visitor skips yet; the walk honours it\")]
    Skip,
}

/// A visitor of [`walk`].
pub(crate) trait Visitor<'a> {
    /// Called on every message, parents before children, in field order.
    fn enter(&mut self, r: PgRef<'a>) -> Walk;
    /// Called after the children of a message whose `enter` answered [`Walk::Children`].
    fn leave(&mut self, r: PgRef<'a>) {
        let _ = r;
    }
}

/// Visits `root` and every message under it, pre-order, with an explicit stack.
pub(crate) fn walk<'a>(root: PgRef<'a>, v: &mut impl Visitor<'a>) {
    enum Frame<'a> {
        Enter(PgRef<'a>),
        Leave(PgRef<'a>),
    }
    let mut stack = vec![Frame::Enter(root)];
    let mut children = Vec::new();
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Leave(r) => v.leave(r),
            Frame::Enter(r) => {
                if v.enter(r) == Walk::Children {
                    stack.push(Frame::Leave(r));
                    push_children(r, &mut children);
                    stack.extend(children.drain(..).map(Frame::Enter));
                }
            }
        }
    }
}
";

fn rustfmt(src: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning rustfmt")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no rustfmt stdin"))?
        .write_all(src.as_bytes())?;
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success(),
        "rustfmt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

/// Generates the visitor under `root`; with `check`, compares it with the checked-in file
/// instead of writing it. `Ok(false)`: stale.
pub fn run_in(root: &Path, check: bool) -> Result<bool> {
    let proto_path = root.join(PROTO);
    let proto =
        std::fs::read_to_string(&proto_path).with_context(|| proto_path.display().to_string())?;
    let want = generate(&proto)?;
    let out: PathBuf = root.join(OUT);
    if check {
        let have = std::fs::read_to_string(&out).with_context(|| out.display().to_string())?;
        if have.replace("\r\n", "\n") == want {
            println!("gen-pg-visitor: {} is fresh", out.display());
            Ok(true)
        } else {
            eprintln!(
                "gen-pg-visitor: {} is stale; run `cargo xtask gen-pg-visitor`",
                out.display()
            );
            Ok(false)
        }
    } else {
        std::fs::write(&out, want).with_context(|| out.display().to_string())?;
        println!("gen-pg-visitor: wrote {}", out.display());
        Ok(true)
    }
}

fn parse_args(args: &[String]) -> Result<bool> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        _ => bail!("usage: xtask gen-pg-visitor [--check]"),
    }
}

/// `cargo xtask gen-pg-visitor [--check]`.
pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let result = parse_args(&args).and_then(|check| run_in(&crate::workspace_root(), check));
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("xtask gen-pg-visitor: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;

    #[test]
    fn names_convert_as_prost_converts_them() {
        for (proto, rust) in [
            ("SQLValueFunction", "SqlValueFunction"),
            ("A_Const", "AConst"),
            ("A_ArrayExpr", "AArrayExpr"),
            ("CTESearchClause", "CteSearchClause"),
            ("PLAssignStmt", "PlAssignStmt"),
            ("RTEPermissionInfo", "RtePermissionInfo"),
            ("ATAlterConstraint", "AtAlterConstraint"),
            ("AlterTSDictionaryStmt", "AlterTsDictionaryStmt"),
            ("CoerceViaIO", "CoerceViaIo"),
            ("sqlvalue_function", "SqlvalueFunction"),
            ("alter_tsdictionary_stmt", "AlterTsdictionaryStmt"),
            ("RawStmt", "RawStmt"),
        ] {
            assert_eq!(upper_camel(proto), rust, "{proto}");
        }
        assert_eq!(snake("A_Const"), "a_const");
        assert_eq!(snake("SummaryResult"), "summary_result");
        assert_eq!(field_ident("type"), "r#type");
        assert_eq!(field_ident("override"), "r#override");
        assert_eq!(field_ident("relname"), "relname");
    }

    #[test]
    fn the_reader_takes_fields_oneofs_and_repeated_fields() {
        let s = parse_schema(
            "syntax = \"proto3\"; package p; // c\n\
             message A { B b = 1 [json_name=\"b\"]; repeated B bs = 2; /* x */ int32 n = 3; \
             oneof v { B one = 4; string s = 5; } }\n\
             message B { E e = 1; }\n\
             enum E { X = 0; }\n",
        )
        .unwrap();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].members.len(), 4);
        assert!(matches!(&s.messages[0].members[1], Member::Field(f) if f.repeated && f.ty == "B"));
        assert!(
            matches!(&s.messages[0].members[3], Member::Oneof(o) if o.arms.len() == 2 && o.name == "v")
        );
        assert!(s.enums.contains("E"));
    }

    #[test]
    fn an_unknown_construct_or_type_is_an_error() {
        assert!(parse_schema("syntax = \"proto3\"; service S {}").is_err());
        assert!(parse_schema("syntax = \"proto2\";").is_err());
        assert!(parse_schema("message A { map<string, string> m = 1; }").is_err());
        let unknown = "syntax = \"proto3\"; message Node { oneof node { A a = 1; } } \
                       message A { Missing m = 1; }";
        assert!(generate(unknown).is_err());
    }

    #[test]
    fn the_checked_in_visitor_is_fresh() {
        assert!(run_in(&workspace_root(), true).unwrap());
    }

    #[test]
    fn a_schema_change_makes_the_visitor_stale() {
        let root = workspace_root();
        let proto = std::fs::read_to_string(root.join(PROTO)).unwrap();
        let changed = proto.replacen(
            "message Alias\n{",
            "message Alias\n{\n  Node extra = 99;",
            1,
        );
        assert_ne!(changed, proto, "the schema still declares Alias");
        let have = std::fs::read_to_string(root.join(OUT)).unwrap();
        assert_ne!(generate(&changed).unwrap(), have.replace("\r\n", "\n"));
    }

    #[test]
    fn arguments() {
        assert!(!parse_args(&[]).unwrap());
        assert!(parse_args(&["--check".to_string()]).unwrap());
        assert!(parse_args(&["--write".to_string()]).is_err());
    }
}
