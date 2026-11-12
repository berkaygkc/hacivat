//! Matching typed text against a template (HV-DRV-03/04): memoised backtracking over the
//! template nodes, in two passes.
//!
//! 1. **Ends.** Every node, at every position, yields the positions where it can end, in
//!    preference order (placeholders are greedy, a `path` is lazy, optional groups try their
//!    alternatives before nothing, lists prefer more items), each with a back-pointer saying how
//!    it got there (which alternative, which list item from which position, where the first node
//!    of a sequence ended). Only the first way to reach each end is kept: what follows depends
//!    only on the position, so this loses no match. Results are memoised per (node, position)
//!    and per (node sequence, position); no bindings are built in this pass, so memory is
//!    linear in the work done.
//! 2. **Bindings.** For the winning whole-input end, the back-pointers are followed once and the
//!    captures of that one path are collected.
//!
//! Limits: the work is capped at 100,000 steps (past the cap the input does not match), a list
//! holds at most 1,024 items, and input over 64 KiB is refused unread.
//!
//! Literals match exactly, except the leading scheme, which matches ASCII-case-insensitively
//! (`JDBC:` is `jdbc:`; never Unicode case folding). A `%`-escaped placeholder is
//! percent-decoded; an invalid escape (`%4G`, a trailing `%`) or bytes that are not UTF-8 mean
//! no match. The whole input must be consumed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;

use percent_encoding::percent_decode_str;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use super::render::{has_anonymous, names};
use super::template::{Escape, Node, PhType, Placeholder};
use super::{FieldId, FieldMap, Template};

/// The longest input the matcher reads.
const MAX_INPUT: usize = 64 * 1024;
/// The most steps one match may take.
const BUDGET: usize = 100_000;
/// The most items one list may match.
pub(super) const MAX_LIST_ITEMS: usize = 1024;

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

/// What the winning path bound.
#[derive(Debug, Default)]
struct Bind {
    named: Vec<(FieldId, Cap)>,
    hosts: Vec<(Cap, Option<Cap>)>,
    extras: Vec<(Cap, Option<Cap>)>,
    /// Anonymous captures not yet paired into an extra.
    anon: Vec<Cap>,
}

impl Bind {
    fn append(&mut self, other: Bind) {
        self.named.extend(other.named);
        self.hosts.extend(other.hosts);
        self.extras.extend(other.extras);
        self.anon.extend(other.anon);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.named.len() + self.hosts.len() + self.extras.len() + self.anon.len()
    }
}

/// How a node reached an end.
#[derive(Clone, Copy, Debug)]
enum Via {
    /// A literal, a placeholder, a not-ahead.
    Plain,
    /// An optional group's alternative.
    Alt(usize),
    /// An optional group or a maybe-empty node, matching nothing.
    Empty,
    /// A maybe-empty node's inner node.
    Inner,
    /// A list item: the end before it, the item alternative, and where the item started.
    Step {
        prev: usize,
        item: usize,
        start: usize,
    },
}

/// A node's ends, in preference order.
type NodeEnds = Rc<Vec<(usize, Via)>>;
/// A sequence's ends, each with where its first node ended.
type SeqEnds = Rc<Vec<(usize, usize)>>;

fn run(t: &Template, input: &str) -> Option<Bind> {
    if input.len() > MAX_INPUT {
        return None;
    }
    let mut m = Matcher::new(input);
    let ends = m.seq(&t.nodes, 0);
    if m.over || !ends.iter().any(|(end, _)| *end == input.len()) {
        return None;
    }
    let mut bind = Bind::default();
    m.build_seq(&t.nodes, 0, input.len(), &mut bind)?;
    Some(bind)
}

struct Matcher<'t, 'i> {
    input: &'i str,
    seq_memo: HashMap<(usize, usize, usize), SeqEnds>,
    node_memo: HashMap<(usize, usize), NodeEnds>,
    steps: usize,
    over: bool,
    /// `bad_pct[i]`: how many `%` before byte `i` do not start a `%XX` escape.
    bad_pct: Vec<u32>,
    _nodes: std::marker::PhantomData<&'t Node>,
}

fn seq_key(nodes: &[Node], pos: usize) -> (usize, usize, usize) {
    (nodes.as_ptr() as usize, nodes.len(), pos)
}

fn node_key(node: &Node, pos: usize) -> (usize, usize) {
    (std::ptr::from_ref(node) as usize, pos)
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
            seq_memo: HashMap::new(),
            node_memo: HashMap::new(),
            steps: 0,
            over: false,
            bad_pct,
            _nodes: std::marker::PhantomData,
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

    // ---- Pass 1: ends ----

    fn seq(&mut self, nodes: &'t [Node], pos: usize) -> SeqEnds {
        let key = seq_key(nodes, pos);
        if let Some(hit) = self.seq_memo.get(&key) {
            return hit.clone();
        }
        let result = if self.over {
            Vec::new()
        } else if let Some((first, rest)) = nodes.split_first() {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            let firsts = self.node(first, pos);
            'outer: for &(e1, _) in firsts.iter() {
                let tail = self.seq(rest, e1);
                for &(e2, _) in tail.iter() {
                    if !self.charge(1) {
                        break 'outer;
                    }
                    if seen.insert(e2) {
                        out.push((e2, e1));
                    }
                }
            }
            out
        } else {
            vec![(pos, pos)]
        };
        let result = Rc::new(result);
        self.seq_memo.insert(key, result.clone());
        result
    }

    fn node(&mut self, node: &'t Node, pos: usize) -> NodeEnds {
        let key = node_key(node, pos);
        if let Some(hit) = self.node_memo.get(&key) {
            return hit.clone();
        }
        let result = Rc::new(self.node_ends(node, pos));
        self.node_memo.insert(key, result.clone());
        result
    }

    fn node_ends(&mut self, node: &'t Node, pos: usize) -> Vec<(usize, Via)> {
        let input = self.input;
        let rest = &input.as_bytes()[pos..];
        match node {
            Node::Lit(s) => {
                if rest.starts_with(s.as_bytes()) {
                    vec![(pos + s.len(), Via::Plain)]
                } else {
                    Vec::new()
                }
            }
            Node::Scheme(s) => {
                if rest.len() >= s.len() && rest[..s.len()].eq_ignore_ascii_case(s.as_bytes()) {
                    vec![(pos + s.len(), Via::Plain)]
                } else {
                    Vec::new()
                }
            }
            Node::NotAhead(s) => {
                if rest.starts_with(s.as_bytes()) {
                    Vec::new()
                } else {
                    vec![(pos, Via::Plain)]
                }
            }
            Node::MaybeEmpty(inner) => {
                let mut out: Vec<(usize, Via)> = self
                    .node(inner, pos)
                    .iter()
                    .map(|&(e, _)| (e, Via::Inner))
                    .collect();
                if !out.iter().any(|(e, _)| *e == pos) {
                    out.push((pos, Via::Empty));
                }
                out
            }
            Node::Opt(alts) => {
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for (i, alt) in alts.iter().enumerate() {
                    for &(e, _) in self.seq(alt, pos).iter() {
                        if seen.insert(e) {
                            out.push((e, Via::Alt(i)));
                        }
                    }
                }
                if seen.insert(pos) {
                    out.push((pos, Via::Empty));
                }
                out
            }
            Node::List { sep, items } => self.list(sep, items, pos),
            Node::Ph(p) => self.placeholder(p, pos),
        }
    }

    /// `item (sep item)*` with at most `MAX_LIST_ITEMS` items, or nothing; more items first.
    fn list(&mut self, sep: &str, items: &'t [Vec<Node>], pos: usize) -> Vec<(usize, Via)> {
        let mut seen = HashSet::from([pos]);
        let mut found: Vec<(usize, Via)> = Vec::new();
        let mut frontier = VecDeque::from([(pos, 0usize)]);
        while let Some((at, count)) = frontier.pop_front() {
            if count >= MAX_LIST_ITEMS {
                continue;
            }
            let start = if at == pos {
                pos
            } else if self.input[at..].starts_with(sep) {
                at + sep.len()
            } else {
                continue;
            };
            for (idx, item) in items.iter().enumerate() {
                let ends = self.seq(item, start);
                for &(e, _) in ends.iter() {
                    if !self.charge(1) {
                        return Vec::new();
                    }
                    // An item must consume text, or the list could repeat forever.
                    if e == start || !seen.insert(e) {
                        continue;
                    }
                    found.push((
                        e,
                        Via::Step {
                            prev: at,
                            item: idx,
                            start,
                        },
                    ));
                    frontier.push_back((e, count + 1));
                }
            }
        }
        found.reverse();
        found.push((pos, Via::Empty));
        found
    }

    fn placeholder(&mut self, p: &'t Placeholder, pos: usize) -> Vec<(usize, Via)> {
        let input = self.input;
        let mut out = Vec::new();
        // A bracketed IPv6 literal for a host.
        if let Some(close) = self.bracketed_host(p, pos) {
            out.push((close + 1, Via::Plain));
        }
        let mut ends: Vec<usize> = match &p.regex {
            Some(_) => {
                let mut ends: Vec<usize> = input[pos..]
                    .char_indices()
                    .map(|(i, c)| pos + i + c.len_utf8())
                    .collect();
                ends.insert(0, pos);
                ends
            }
            None => {
                let mut ends = vec![pos];
                for (n, (i, c)) in input[pos..].char_indices().enumerate() {
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
                && !re.re.is_match(&input[r])
            {
                continue;
            }
            out.push((end, Via::Plain));
        }
        out
    }

    /// For a host placeholder at `[`, the position of the `]` closing an IPv6 literal.
    fn bracketed_host(&self, p: &Placeholder, pos: usize) -> Option<usize> {
        if p.ty != PhType::Host || p.regex.is_some() || !self.input[pos..].starts_with('[') {
            return None;
        }
        let inner = pos + 1;
        let len = self.input[inner..]
            .find(|c: char| !PhType::Ipv6.allows(c))
            .unwrap_or(self.input.len() - inner);
        let close = inner + len;
        (len > 0 && self.input[close..].starts_with(']') && self.escapes_ok(&(inner..close)))
            .then_some(close)
    }

    // ---- Pass 2: the bindings of one path ----

    /// Collects the captures of `nodes` from `pos` to `end` into `bind`.
    fn build_seq(&self, nodes: &'t [Node], pos: usize, end: usize, bind: &mut Bind) -> Option<()> {
        let mut at = pos;
        for i in 0..nodes.len() {
            let ends = self.seq_memo.get(&seq_key(&nodes[i..], at))?;
            let &(_, first_end) = ends.iter().find(|(e, _)| *e == end)?;
            self.build_node(&nodes[i], at, first_end, bind)?;
            at = first_end;
        }
        (at == end).then_some(())
    }

    fn via(&self, node: &Node, pos: usize, end: usize) -> Option<Via> {
        let ends = self.node_memo.get(&node_key(node, pos))?;
        ends.iter().find(|(e, _)| *e == end).map(|&(_, v)| v)
    }

    fn build_node(&self, node: &'t Node, pos: usize, end: usize, bind: &mut Bind) -> Option<()> {
        match node {
            Node::Lit(_) | Node::Scheme(_) | Node::NotAhead(_) => {}
            Node::Ph(p) => {
                let value = if self.bracketed_host(p, pos) == Some(end.wrapping_sub(1)) {
                    pos + 1..end - 1
                } else {
                    pos..end
                };
                let cap = Cap {
                    span: pos..end,
                    value,
                    escape: p.escape,
                };
                match p.name {
                    Some(f) => bind.named.push((f, cap)),
                    None => bind.anon.push(cap),
                }
            }
            Node::MaybeEmpty(inner) => {
                if let Via::Inner = self.via(node, pos, end)? {
                    self.build_node(inner, pos, end, bind)?;
                }
            }
            Node::Opt(alts) => {
                if let Via::Alt(i) = self.via(node, pos, end)? {
                    self.build_seq(&alts[i], pos, end, bind)?;
                }
            }
            Node::List { items, .. } => {
                let mut steps = Vec::new();
                let mut cur = end;
                while cur != pos {
                    let Via::Step { prev, item, start } = self.via(node, pos, cur)? else {
                        return None;
                    };
                    steps.push((item, start, cur));
                    cur = prev;
                }
                for (item, start, item_end) in steps.into_iter().rev() {
                    let seq = &items[item];
                    let mut ib = Bind::default();
                    self.build_seq(seq, start, item_end, &mut ib)?;
                    bind.append(item_bind(ib, names(seq, FieldId::Host), has_anonymous(seq)));
                }
            }
        }
        Some(())
    }
}

/// An item's bindings as list entries: a host item's host and port become a host entry, a
/// pass-through item's key and value an extra.
fn item_bind(mut b: Bind, host_item: bool, pass_item: bool) -> Bind {
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
    b.anon.clear();
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

    fn query(n: usize) -> String {
        let params: Vec<String> = (0..n).map(|i| format!("k{i}=v{i}")).collect();
        format!("q://h?{}", params.join("&"))
    }

    #[test]
    fn memo_and_bindings_stay_linear_in_the_work() {
        let t = compile(r"q://{host}[\?<&,{:ident}={:param}>]", "x").unwrap();
        let input = query(MAX_LIST_ITEMS);
        let mut m = Matcher::new(&input);
        let ends = m.seq(&t.nodes, 0);
        assert!(!m.over, "{} steps", m.steps);
        assert!(ends.iter().any(|(e, _)| *e == input.len()));
        let stored: usize = m.seq_memo.values().map(|v| v.len()).sum::<usize>()
            + m.node_memo.values().map(|v| v.len()).sum::<usize>();
        // Every stored end was charged as a step, so the memo cannot outgrow the budget.
        assert!(
            stored <= m.steps + m.seq_memo.len() + m.node_memo.len(),
            "{stored}"
        );
        let mut bind = Bind::default();
        m.build_seq(&t.nodes, 0, input.len(), &mut bind).unwrap();
        // One host and one extra per item: nothing accumulated per reached end.
        assert_eq!(bind.extras.len(), MAX_LIST_ITEMS);
        assert_eq!(bind.len(), MAX_LIST_ITEMS + 1);
    }

    #[test]
    fn a_list_stops_at_its_item_cap() {
        let t = compile(r"q://{host}[\?<&,{:ident}={:param}>]", "x").unwrap();
        assert_eq!(
            match_template(&t, &query(MAX_LIST_ITEMS)).unwrap().1.len(),
            MAX_LIST_ITEMS
        );
        assert!(match_template(&t, &query(MAX_LIST_ITEMS + 1)).is_none());
    }
}
