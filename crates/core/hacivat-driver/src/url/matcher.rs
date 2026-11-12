//! Matching typed text against a template (HV-DRV-03/04): memoised backtracking over the
//! template nodes.
//!
//! - Every node, at every position, yields the positions where it can end, in preference
//!   order (placeholders are greedy, a `path` is lazy, optional groups try their alternatives
//!   before nothing, lists prefer more items). Only the first way to reach each end is kept:
//!   what follows depends only on the position, so this loses no match.
//! - Results are memoised per (node sequence, position), and the work is capped at 100,000
//!   steps; past the cap the input does not match. Input over 64 KiB is refused unread.
//! - Literals match exactly, except the leading scheme, which matches ASCII-case-insensitively
//!   (`JDBC:` is `jdbc:`; never Unicode case folding).
//! - A `%`-escaped placeholder is percent-decoded; an invalid escape (`%4G`, a trailing `%`) or
//!   bytes that are not UTF-8 mean no match.
//! - The whole input must be consumed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;

use percent_encoding::percent_decode_str;
use regex::{Regex, RegexBuilder};
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use super::render::{has_anonymous, names};
use super::template::{Escape, Node, PhType, Placeholder, REGEX_SIZE_LIMIT};
use super::{FieldId, FieldMap, Template};

/// The longest input the matcher reads.
const MAX_INPUT: usize = 64 * 1024;
/// The most steps one match may take.
const BUDGET: usize = 100_000;

/// The fields and extra parameters of `input`, if it matches `t`.
///
/// `FieldMap` never holds a password, so a password matched by a `{password}` placeholder is
/// dropped here; [`match_template_with_password`] returns it as a secret. `hosts` is filled
/// only by a host list; its first entry is also `values[Host]` and `values[Port]`.
pub fn match_template(t: &Template, input: &str) -> Option<(FieldMap, Vec<(String, String)>)> {
    match_template_with_password(t, input).map(|(map, extra, _)| (map, extra))
}

/// A match: the fields, the extra parameters, and the password as a secret.
pub(crate) type Matched = (FieldMap, Vec<(String, String)>, Option<SecretString>);

/// [`match_template`], plus the matched password as a secret (for `parse`, which moves it to
/// the secret store). The decoded text is wiped once copied into the secret.
pub(crate) fn match_template_with_password(t: &Template, input: &str) -> Option<Matched> {
    let bind = run(t, input)?;
    let mut map = FieldMap::default();
    let mut password = None;
    for (f, cap) in &bind.named {
        let value = Zeroizing::new(cap.decode(input)?);
        if *f == FieldId::Password {
            password = Some(SecretString::from(value.as_str()));
        } else {
            map.values.insert(*f, value.as_str().to_string());
        }
    }
    for (h, p) in &bind.hosts {
        let host = h.decode(input)?;
        let port = match p {
            Some(p) => Some(p.decode(input)?),
            None => None,
        };
        let port_num = match &port {
            Some(p) => Some(p.parse::<u16>().ok()?),
            None => None,
        };
        if map.hosts.is_empty() {
            map.values
                .entry(FieldId::Host)
                .or_insert_with(|| host.clone());
            if let Some(p) = port {
                map.values.entry(FieldId::Port).or_insert(p);
            }
        }
        map.hosts.push((host, port_num));
    }
    let mut extra = Vec::with_capacity(bind.extras.len());
    for (k, v) in &bind.extras {
        let value = match v {
            Some(v) => v.decode(input)?,
            None => String::new(),
        };
        extra.push((k.decode(input)?, value));
    }
    Some((map, extra, password))
}

/// Where the fields of a match sit in the input, for an endpoint rewrite.
pub(super) struct Spans {
    /// Named placeholders outside host lists, with the whole span (an IPv6 host includes its
    /// brackets).
    pub(super) named: Vec<(FieldId, Range<usize>)>,
    /// Host-list entries: the host span and the port span.
    pub(super) hosts: Vec<(Range<usize>, Option<Range<usize>>)>,
}

/// The spans of `input` matched against `t`.
pub(super) fn match_spans(t: &Template, input: &str) -> Option<Spans> {
    let bind = run(t, input)?;
    Some(Spans {
        named: bind
            .named
            .iter()
            .map(|(f, c)| (*f, c.span.clone()))
            .collect(),
        hosts: bind
            .hosts
            .iter()
            .map(|(h, p)| (h.span.clone(), p.as_ref().map(|p| p.span.clone())))
            .collect(),
    })
}

/// A captured placeholder: the text it covers, and the part that holds the value.
#[derive(Clone, Debug)]
struct Cap {
    span: Range<usize>,
    value: Range<usize>,
    escape: Escape,
}

impl Cap {
    fn decode(&self, input: &str) -> Option<String> {
        let raw = &input[self.value.clone()];
        match self.escape {
            Escape::None => Some(raw.to_string()),
            Escape::Percent => percent_decode_str(raw)
                .decode_utf8()
                .ok()
                .map(|c| c.into_owned()),
        }
    }
}

/// What a partial match bound.
#[derive(Clone, Debug, Default)]
struct Bind {
    named: Vec<(FieldId, Cap)>,
    hosts: Vec<(Cap, Option<Cap>)>,
    extras: Vec<(Cap, Option<Cap>)>,
    /// Anonymous captures not yet paired into an extra.
    anon: Vec<Cap>,
}

impl Bind {
    fn concat(&self, other: &Bind) -> Bind {
        let mut b = self.clone();
        b.named.extend(other.named.iter().cloned());
        b.hosts.extend(other.hosts.iter().cloned());
        b.extras.extend(other.extras.iter().cloned());
        b.anon.extend(other.anon.iter().cloned());
        b
    }
}

type Ends = Rc<Vec<(usize, Bind)>>;

fn run(t: &Template, input: &str) -> Option<Bind> {
    if input.len() > MAX_INPUT {
        return None;
    }
    let mut m = Matcher::new(input);
    let ends = m.seq(&t.nodes, 0);
    if m.over {
        return None;
    }
    ends.iter()
        .find(|(end, _)| *end == input.len())
        .map(|(_, b)| b.clone())
}

struct Matcher<'t, 'i> {
    input: &'i str,
    memo: HashMap<(usize, usize, usize), Ends>,
    steps: usize,
    over: bool,
    regexes: HashMap<&'t str, Option<Regex>>,
    /// `bad_pct[i]`: how many `%` before byte `i` do not start a `%XX` escape.
    bad_pct: Vec<u32>,
}

impl<'t, 'i> Matcher<'t, 'i> {
    fn new(input: &'i str) -> Self {
        let b = input.as_bytes();
        let mut bad_pct = Vec::with_capacity(b.len() + 1);
        let mut n = 0u32;
        bad_pct.push(0);
        for i in 0..b.len() {
            let ok = b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
            if b[i] == b'%' && !ok {
                n += 1;
            }
            bad_pct.push(n);
        }
        Matcher {
            input,
            memo: HashMap::new(),
            steps: 0,
            over: false,
            regexes: HashMap::new(),
            bad_pct,
        }
    }

    /// Charges `n` steps; `false` once the budget is spent.
    fn charge(&mut self, n: usize) -> bool {
        self.steps += n;
        if self.steps > BUDGET {
            self.over = true;
        }
        !self.over
    }

    /// Every `%` in `r` starts a `%XX` escape that ends inside `r`.
    fn escapes_ok(&self, r: &Range<usize>) -> bool {
        if self.bad_pct[r.end] != self.bad_pct[r.start] {
            return false;
        }
        let b = self.input.as_bytes();
        let tail = r.end.saturating_sub(2).max(r.start);
        !b[tail..r.end].contains(&b'%')
    }

    fn seq(&mut self, nodes: &'t [Node], pos: usize) -> Ends {
        let key = (nodes.as_ptr() as usize, nodes.len(), pos);
        if let Some(hit) = self.memo.get(&key) {
            return hit.clone();
        }
        let result = if self.over {
            Vec::new()
        } else if let Some((first, rest)) = nodes.split_first() {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            'outer: for (e1, b1) in self.node(first, pos) {
                let tail = self.seq(rest, e1);
                for (e2, b2) in tail.iter() {
                    if !self.charge(1) {
                        break 'outer;
                    }
                    if seen.insert(*e2) {
                        out.push((*e2, b1.concat(b2)));
                    }
                }
            }
            out
        } else {
            vec![(pos, Bind::default())]
        };
        let result = Rc::new(result);
        self.memo.insert(key, result.clone());
        result
    }

    fn node(&mut self, node: &'t Node, pos: usize) -> Vec<(usize, Bind)> {
        let input = self.input;
        let rest = &input.as_bytes()[pos..];
        match node {
            Node::Lit(s) => {
                if rest.starts_with(s.as_bytes()) {
                    vec![(pos + s.len(), Bind::default())]
                } else {
                    Vec::new()
                }
            }
            Node::Scheme(s) => {
                if rest.len() >= s.len() && rest[..s.len()].eq_ignore_ascii_case(s.as_bytes()) {
                    vec![(pos + s.len(), Bind::default())]
                } else {
                    Vec::new()
                }
            }
            Node::NotAhead(s) => {
                if rest.starts_with(s.as_bytes()) {
                    Vec::new()
                } else {
                    vec![(pos, Bind::default())]
                }
            }
            Node::MaybeEmpty(inner) => {
                let mut out = self.node(inner, pos);
                if !out.iter().any(|(e, _)| *e == pos) {
                    out.push((pos, Bind::default()));
                }
                out
            }
            Node::Opt(alts) => {
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for alt in alts {
                    for (e, b) in self.seq(alt, pos).iter() {
                        if seen.insert(*e) {
                            out.push((*e, b.clone()));
                        }
                    }
                }
                if seen.insert(pos) {
                    out.push((pos, Bind::default()));
                }
                out
            }
            Node::List { sep, items } => self.list(sep, items, pos),
            Node::Ph(p) => self.placeholder(p, pos),
        }
    }

    /// `item (sep item)*`, or nothing; more items first.
    fn list(&mut self, sep: &str, items: &'t [Vec<Node>], pos: usize) -> Vec<(usize, Bind)> {
        let mut seen = HashSet::from([pos]);
        let mut found: Vec<(usize, Bind)> = Vec::new();
        let mut frontier = VecDeque::from([(pos, Bind::default())]);
        while let Some((at, bind)) = frontier.pop_front() {
            let start = if at == pos {
                pos
            } else if self.input[at..].starts_with(sep) {
                at + sep.len()
            } else {
                continue;
            };
            for item in items {
                let host_item = names(item, FieldId::Host);
                let pass_item = has_anonymous(item);
                let ends = self.seq(item, start);
                for (e, ib) in ends.iter() {
                    if !self.charge(1) {
                        return Vec::new();
                    }
                    // An item must consume text, or the list could repeat forever.
                    if *e == start || !seen.insert(*e) {
                        continue;
                    }
                    let mut b = bind.concat(&item_bind(ib, host_item, pass_item));
                    b.anon.clear();
                    found.push((*e, b.clone()));
                    frontier.push_back((*e, b));
                }
            }
        }
        found.reverse();
        found.push((pos, Bind::default()));
        found
    }

    fn placeholder(&mut self, p: &'t Placeholder, pos: usize) -> Vec<(usize, Bind)> {
        let input = self.input;
        let mut out = Vec::new();
        let bind = |span: Range<usize>, value: Range<usize>| {
            let cap = Cap {
                span,
                value,
                escape: p.escape,
            };
            let mut b = Bind::default();
            match p.name {
                Some(f) => b.named.push((f, cap)),
                None => b.anon.push(cap),
            }
            b
        };
        // A bracketed IPv6 literal for a host.
        if p.ty == PhType::Host && p.regex.is_none() && self.input[pos..].starts_with('[') {
            let inner = pos + 1;
            let len = self.input[inner..]
                .find(|c: char| !PhType::Ipv6.allows(c))
                .unwrap_or(self.input.len() - inner);
            let close = inner + len;
            let value = inner..close;
            if len > 0 && self.input[close..].starts_with(']') && self.escapes_ok(&value) {
                out.push((close + 1, bind(pos..close + 1, value)));
            }
        }
        let mut ends: Vec<usize> = match &p.regex {
            Some(_) => {
                let mut ends: Vec<usize> = self.input[pos..]
                    .char_indices()
                    .map(|(i, c)| pos + i + c.len_utf8())
                    .collect();
                ends.insert(0, pos);
                ends
            }
            None => {
                let mut ends = vec![pos];
                for (n, (i, c)) in self.input[pos..].char_indices().enumerate() {
                    if n >= p.ty.max_len() || !p.ty.allows(c) {
                        break;
                    }
                    ends.push(pos + i + c.len_utf8());
                }
                let min = p.ty.min_len();
                if ends.len() > min {
                    ends.drain(..min);
                } else {
                    ends.clear();
                }
                ends
            }
        };
        if !p.ty.lazy() {
            ends.reverse();
        }
        for end in ends {
            let r = pos..end;
            let cost = 1 + if p.regex.is_some() { r.len() / 64 } else { 0 };
            if !self.charge(cost) {
                return Vec::new();
            }
            if p.escape == Escape::Percent && !self.escapes_ok(&r) {
                continue;
            }
            if let Some(re) = &p.regex
                && !self
                    .regex(re)
                    .is_some_and(|re| re.is_match(&input[r.clone()]))
            {
                continue;
            }
            out.push((end, bind(r.clone(), r)));
        }
        out
    }

    fn regex(&mut self, src: &'t str) -> Option<&Regex> {
        self.regexes
            .entry(src)
            .or_insert_with(|| {
                RegexBuilder::new(&format!("^(?:{src})$"))
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .ok()
            })
            .as_ref()
    }
}

/// An item's bindings as list entries: a host item's host and port become a host entry, a
/// pass-through item's key and value an extra.
fn item_bind(ib: &Bind, host_item: bool, pass_item: bool) -> Bind {
    let mut b = ib.clone();
    if host_item {
        let take = |b: &mut Bind, f: FieldId| {
            b.named
                .iter()
                .position(|(n, _)| *n == f)
                .map(|i| b.named.remove(i).1)
        };
        if let Some(host) = take(&mut b, FieldId::Host) {
            let port = take(&mut b, FieldId::Port);
            b.hosts.push((host, port));
        }
    }
    if pass_item {
        let mut anon = std::mem::take(&mut b.anon).into_iter();
        if let Some(key) = anon.next() {
            b.extras.push((key, anon.next()));
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::url::compile;

    #[test]
    fn the_password_comes_back_only_as_a_secret() {
        let t = compile("pg://[{user}[:{password}]@]{host}", "x").unwrap();
        let (map, _, pw) = match_template_with_password(&t, "pg://u:s3cr%40t@h").unwrap();
        assert_eq!(pw.unwrap().expose_secret(), "s3cr@t");
        assert_eq!(map.values[&FieldId::User], "u");
        assert!(!map.values.contains_key(&FieldId::Password));
        let (_, _, none) = match_template_with_password(&t, "pg://u@h").unwrap();
        assert!(none.is_none());
    }
}
