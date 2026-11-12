//! URL templates: compile DataGrip-style template text into nodes (HV-DRV-02).
//!
//! Grammar (alg/01 HV-DRV-02):
//! ```text
//! template    := node*
//! node        := literal | escape | placeholder | optional | list | notahead | node '?'
//! escape      := '\' any
//! placeholder := '{' name? (':' type ('/' regex)?)? (':' default)? (':' escape_mode)? '}'
//! type        := host | port | ident | param | path | file | ipv6 | text
//! escape_mode := '%' (percent-encode and decode, the default) | 'none'
//! optional    := '[' seq ('|' seq)* ']'
//! list        := '<' sep ',' seq (',' seq)* '>'      each seq is an item alternative
//! notahead    := '!(' literal_text ')'
//! ```
//! Inside a placeholder, `\:`, `\/`, `\{` and `\}` stand for the bare character (so a regex
//! quantifier is written `\{1,5\}`); every other escape is kept for the regex. An unescaped `?`
//! after a node makes it optional-empty, so a literal `?` is `\?`. A type with no name is
//! inferred from the field: `host`, `port`, `file`, `socket` (a `path`), else `text`.
//!
//! The scheme at the start of a template (`jdbc:mysql:`, `postgresql:`) is kept as its own
//! node, because it is the only literal matched ASCII-case-insensitively.

use std::collections::BTreeSet;

use percent_encoding::{AsciiSet, CONTROLS};
use regex::{Regex, RegexBuilder};

use super::FieldId;

/// The size limit for a placeholder regex.
pub(super) const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A compiled URL template (HV-DRV-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Template {
    pub id: String,
    pub field_set: BTreeSet<FieldId>,
    /// The template ends in a pass-through of extra parameters.
    pub has_passthrough: bool,
    pub(super) nodes: Vec<Node>,
}

/// A template node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Node {
    Lit(String),
    /// The leading scheme (`jdbc:mysql:`), matched ASCII-case-insensitively.
    Scheme(String),
    Ph(Placeholder),
    /// Alternatives, tried in order; none of them may render.
    Opt(Vec<Vec<Node>>),
    /// Items joined by `sep`; each item alternative is a node sequence.
    List {
        sep: String,
        items: Vec<Vec<Node>>,
    },
    /// Matches empty only where the text does not follow; renders nothing.
    NotAhead(String),
    /// The node, or nothing.
    MaybeEmpty(Box<Node>),
}

/// A placeholder; one with no name is a pass-through key or value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Placeholder {
    pub(super) name: Option<FieldId>,
    pub(super) ty: PhType,
    /// A custom regex for the raw text, compiled once with the template.
    pub(super) regex: Option<PhRegex>,
    pub(super) default: Option<String>,
    pub(super) escape: Escape,
}

/// A placeholder regex: its source as written, and the anchored `^(?:src)$` compiled form.
/// Equality and hashing use the source, so `Template` keeps its derives.
#[derive(Debug, Clone)]
pub(super) struct PhRegex {
    src: String,
    pub(super) re: Regex,
}

impl PartialEq for PhRegex {
    fn eq(&self, other: &Self) -> bool {
        self.src == other.src
    }
}

impl Eq for PhRegex {}

impl std::hash::Hash for PhRegex {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.src.hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Escape {
    Percent,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum PhType {
    Host,
    Port,
    Ident,
    Param,
    Path,
    File,
    Ipv6,
    Text,
}

/// Always percent-encoded in a URL component: controls, space and `@ / ? # % : [ ] ;`
/// (non-ASCII bytes are always encoded).
const BASE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'@')
    .add(b'/')
    .add(b'?')
    .add(b'#')
    .add(b'%')
    .add(b':')
    .add(b'[')
    .add(b']')
    .add(b';');
/// A host also stops at `,` (host lists).
const HOST: &AsciiSet = &BASE.add(b',');
/// A parameter key also stops at `&`, `=` and `,`. `+` is encoded in keys and values because
/// form-style decoders (`mysql_async` among them) read a bare `+` as a space.
const IDENT: &AsciiSet = &BASE.add(b'&').add(b'=').add(b',').add(b'+');
/// A parameter value also stops at `&`; `+` is encoded as for a key.
const PARAM: &AsciiSet = &BASE.add(b'&').add(b'+');
/// A path keeps `/`, `:` and `\` readable; it encodes what would end it in a URL.
const PATH: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'?')
    .add(b';')
    .add(b'[')
    .add(b']');
/// An IPv6 literal keeps its `:`.
const IPV6: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'@')
    .add(b'/')
    .add(b'?')
    .add(b'#')
    .add(b'%')
    .add(b'[')
    .add(b']')
    .add(b';');

impl PhType {
    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "host" => PhType::Host,
            "port" => PhType::Port,
            "ident" => PhType::Ident,
            "param" => PhType::Param,
            "path" => PhType::Path,
            "file" => PhType::File,
            "ipv6" => PhType::Ipv6,
            "text" => PhType::Text,
            _ => return None,
        })
    }

    fn for_field(f: Option<FieldId>) -> Self {
        match f {
            Some(FieldId::Host) => PhType::Host,
            Some(FieldId::Port) => PhType::Port,
            Some(FieldId::File) => PhType::File,
            Some(FieldId::Socket) => PhType::Path,
            _ => PhType::Text,
        }
    }

    /// What `%` escaping encodes for this type.
    pub(super) fn encode_set(self) -> &'static AsciiSet {
        match self {
            PhType::Host => HOST,
            PhType::Ident => IDENT,
            PhType::Param => PARAM,
            PhType::Path | PhType::File => PATH,
            PhType::Ipv6 => IPV6,
            PhType::Port | PhType::Text => BASE,
        }
    }

    /// Whether `c` may appear in the raw text of this type (the default regexes of HV-DRV-02,
    /// with whitespace also ending a host, a key or a text field).
    pub(super) fn allows(self, c: char) -> bool {
        match self {
            PhType::Host => !c.is_whitespace() && !":/?#;,[]@".contains(c),
            PhType::Port => c.is_ascii_digit(),
            PhType::Ident => !c.is_whitespace() && !"&;=,:/".contains(c),
            PhType::Param => !"&;".contains(c),
            PhType::Path | PhType::File => c != '\n',
            PhType::Ipv6 => c.is_ascii_hexdigit() || ":.%".contains(c),
            PhType::Text => !c.is_whitespace() && !"@/?#:[];".contains(c),
        }
    }

    /// The most characters a value of this type may have.
    pub(super) fn max_len(self) -> usize {
        match self {
            PhType::Port => 5,
            _ => usize::MAX,
        }
    }

    /// A parameter value may be empty; every other type needs a character.
    pub(super) fn min_len(self) -> usize {
        match self {
            PhType::Param => 0,
            _ => 1,
        }
    }

    /// A path is lazy (`.+?`): the shortest match is tried first.
    pub(super) fn lazy(self) -> bool {
        self == PhType::Path
    }
}

impl Template {
    /// The template's default for `f` (`{host::localhost}` gives `localhost`): the first
    /// placeholder for `f` that has one.
    pub fn default_of(&self, f: FieldId) -> Option<&str> {
        fn find(nodes: &[Node], f: FieldId) -> Option<&str> {
            nodes.iter().find_map(|n| match n {
                Node::Ph(p) if p.name == Some(f) => p.default.as_deref(),
                Node::Opt(alts) | Node::List { items: alts, .. } => {
                    alts.iter().find_map(|seq| find(seq, f))
                }
                Node::MaybeEmpty(inner) => find(std::slice::from_ref(inner.as_ref()), f),
                _ => None,
            })
        }
        find(&self.nodes, f)
    }
}

/// Why a template failed to compile (HV-DRV-02). Positions are byte offsets in the source:
/// `Unbalanced` points at the bracket left open (or the stray closer), the others at the `{` of
/// the placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemplateError {
    Unbalanced {
        pos: usize,
    },
    /// The regex does not compile, or is over the size limit.
    BadRegex {
        pos: usize,
    },
    UnknownField {
        name: String,
    },
    /// More than name, type, default and escape mode.
    TooManyFields {
        pos: usize,
    },
    /// A type name outside `host port ident param path file ipv6 text`.
    UnknownType {
        pos: usize,
    },
    /// An escape mode other than `%` or `none`.
    BadEscapeMode {
        pos: usize,
    },
}

/// Compiles template text `src` as template `id`. The empty template is legal ("URL only").
pub fn compile(src: &str, id: &str) -> Result<Template, TemplateError> {
    let mut p = Parser { src, pos: 0 };
    let mut nodes = p.seq(&[])?;
    split_scheme(&mut nodes);
    let mut field_set = BTreeSet::new();
    let mut has_passthrough = false;
    collect(&nodes, false, &mut field_set, &mut has_passthrough);
    Ok(Template {
        id: id.to_string(),
        field_set,
        has_passthrough,
        nodes,
    })
}

/// Records named fields, and whether a list item has an anonymous placeholder.
fn collect(nodes: &[Node], in_item: bool, fields: &mut BTreeSet<FieldId>, pass: &mut bool) {
    for n in nodes {
        match n {
            Node::Ph(p) => match p.name {
                Some(f) => {
                    fields.insert(f);
                }
                None if in_item => *pass = true,
                None => {}
            },
            Node::Opt(alts) => alts.iter().for_each(|s| collect(s, in_item, fields, pass)),
            Node::List { items, .. } => items.iter().for_each(|s| collect(s, true, fields, pass)),
            Node::MaybeEmpty(inner) => {
                collect(std::slice::from_ref(inner.as_ref()), in_item, fields, pass)
            }
            Node::Lit(_) | Node::Scheme(_) | Node::NotAhead(_) => {}
        }
    }
}

/// Splits a leading `scheme:` (letters, digits, `+ . - :`, starting with a letter, up to its
/// last `:`) off the first literal.
fn split_scheme(nodes: &mut Vec<Node>) {
    let Some(Node::Lit(first)) = nodes.first() else {
        return;
    };
    if !first.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return;
    }
    let run = first
        .find(|c: char| !(c.is_ascii_alphanumeric() || "+.-:".contains(c)))
        .unwrap_or(first.len());
    let Some(colon) = first[..run].rfind(':') else {
        return;
    };
    let scheme = first[..=colon].to_string();
    let rest = first[colon + 1..].to_string();
    nodes[0] = Node::Scheme(scheme);
    if !rest.is_empty() {
        nodes.insert(1, Node::Lit(rest));
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// A node sequence up to (not including) a character of `stops`, or the end.
    fn seq(&mut self, stops: &[char]) -> Result<Vec<Node>, TemplateError> {
        let mut nodes = Vec::new();
        let mut lit = String::new();
        while let Some(c) = self.peek() {
            if stops.contains(&c) {
                break;
            }
            let at = self.pos;
            let node = match c {
                '\\' => {
                    self.bump();
                    lit.push(self.bump().unwrap_or('\\'));
                    continue;
                }
                '{' => self.placeholder()?,
                '[' => self.optional()?,
                '<' => self.list()?,
                '!' if self.src[at..].starts_with("!(") => self.not_ahead()?,
                ']' | '}' => return Err(TemplateError::Unbalanced { pos: at }),
                '?' => {
                    self.bump();
                    // `?` wraps the node before it: a structural node, or the last character.
                    let prev = match lit.pop() {
                        Some(ch) => Node::Lit(ch.to_string()),
                        None => match nodes.pop() {
                            Some(n) => n,
                            None => {
                                lit.push('?');
                                continue;
                            }
                        },
                    };
                    flush(&mut lit, &mut nodes);
                    nodes.push(Node::MaybeEmpty(Box::new(prev)));
                    continue;
                }
                _ => {
                    self.bump();
                    lit.push(c);
                    continue;
                }
            };
            flush(&mut lit, &mut nodes);
            nodes.push(node);
        }
        flush(&mut lit, &mut nodes);
        Ok(nodes)
    }

    fn optional(&mut self) -> Result<Node, TemplateError> {
        let open = self.pos;
        self.bump();
        let mut alts = Vec::new();
        loop {
            alts.push(self.seq(&[']', '|'])?);
            match self.bump() {
                Some('|') => continue,
                Some(']') => return Ok(Node::Opt(alts)),
                _ => return Err(TemplateError::Unbalanced { pos: open }),
            }
        }
    }

    fn list(&mut self) -> Result<Node, TemplateError> {
        let open = self.pos;
        self.bump();
        let mut sep = String::new();
        loop {
            match self.bump() {
                Some('\\') => sep.push(self.bump().unwrap_or('\\')),
                Some(',') => break,
                Some(c) if c != '>' => sep.push(c),
                _ => return Err(TemplateError::Unbalanced { pos: open }),
            }
        }
        let mut items = Vec::new();
        loop {
            items.push(self.seq(&[',', '>'])?);
            match self.bump() {
                Some(',') => continue,
                Some('>') => return Ok(Node::List { sep, items }),
                _ => return Err(TemplateError::Unbalanced { pos: open }),
            }
        }
    }

    fn not_ahead(&mut self) -> Result<Node, TemplateError> {
        let open = self.pos;
        self.pos += 2;
        let mut text = String::new();
        loop {
            match self.bump() {
                Some('\\') => text.push(self.bump().unwrap_or('\\')),
                Some(')') => return Ok(Node::NotAhead(text)),
                Some(c) => text.push(c),
                None => return Err(TemplateError::Unbalanced { pos: open }),
            }
        }
    }

    fn placeholder(&mut self) -> Result<Node, TemplateError> {
        let open = self.pos;
        self.bump();
        // Raw segments split on unescaped `:`, escapes kept for now.
        let mut segments = vec![String::new()];
        loop {
            match self.bump() {
                Some('\\') => {
                    let c = self.bump().unwrap_or('\\');
                    let seg = segments.last_mut().expect("one segment");
                    seg.push('\\');
                    seg.push(c);
                }
                Some(':') => segments.push(String::new()),
                Some('}') => break,
                Some(c) => segments.last_mut().expect("one segment").push(c),
                None => return Err(TemplateError::Unbalanced { pos: open }),
            }
        }
        if segments.len() > 4 {
            return Err(TemplateError::TooManyFields { pos: open });
        }
        let segment = |i: usize| segments.get(i).map(String::as_str).unwrap_or("");

        let name_text = unescape(segment(0));
        let name = if name_text.is_empty() {
            None
        } else {
            Some(
                FieldId::from_id(&name_text)
                    .ok_or(TemplateError::UnknownField { name: name_text })?,
            )
        };

        let (ty_text, regex) = match split_unescaped(segment(1), '/') {
            Some((ty, re)) => (unescape(ty), Some(unescape_regex(re))),
            None => (unescape(segment(1)), None),
        };
        let ty = if ty_text.is_empty() {
            PhType::for_field(name)
        } else {
            PhType::from_id(&ty_text).ok_or(TemplateError::UnknownType { pos: open })?
        };
        let regex = match regex {
            Some(src) => {
                let re = RegexBuilder::new(&format!("^(?:{src})$"))
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|_| TemplateError::BadRegex { pos: open })?;
                Some(PhRegex { src, re })
            }
            None => None,
        };

        let default = Some(unescape(segment(2))).filter(|d| !d.is_empty());
        let escape = match unescape(segment(3)).as_str() {
            "" | "%" => Escape::Percent,
            "none" => Escape::None,
            _ => return Err(TemplateError::BadEscapeMode { pos: open }),
        };
        Ok(Node::Ph(Placeholder {
            name,
            ty,
            regex,
            default,
            escape,
        }))
    }
}

fn flush(lit: &mut String, nodes: &mut Vec<Node>) {
    if !lit.is_empty() {
        nodes.push(Node::Lit(std::mem::take(lit)));
    }
}

/// `\x` becomes `x`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or('\\')
        } else {
            c
        });
    }
    out
}

/// Only `\:`, `\/`, `\{` and `\}` are template escapes in a regex; the rest is regex syntax.
fn unescape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&n)) if ":/{}".contains(n) => {
                out.push(n);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// `s` split at its first unescaped `sep`.
fn split_unescaped(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            return Some((&s[..i], &s[i + c.len_utf8()..]));
        }
    }
    None
}
