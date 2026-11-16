//! `inet`, `cidr`, `macaddr`, `macaddr8`, `bit` and `varbit` in binary, written as
//! PostgreSQL's output functions write them (M3b Task 7, R-M3-67): the canonical text, kept as
//! `Value::Text` under `LogicalType::Network` / `BitString`.
//!
//! - **`inet`/`cidr`** (`network_send`: family, bits, is-cidr, address length, address) as
//!   `network_out` prints them through `pg_inet_net_ntop`: IPv4 always with four octets, IPv6
//!   with the first longest run of two or more zero words shortened to `::` and an embedded IPv4
//!   address for `::a.b.c.d` and `::ffff:a.b.c.d`; `/bits` unless an `inet` has every bit (32 or
//!   128); a `cidr` always has it.
//! - **`macaddr`/`macaddr8`**: lower-case hex pairs joined by `:`.
//! - **`bit`/`varbit`** (`bit_send`: the length in bits, then the bits, most significant
//!   first): one `0` or `1` per bit; the pad bits after the length are not part of the value.
//!
//! Bytes no server sends (an unknown family, bits past the family's width, an address length
//! that is not the family's or the bytes', a cidr flag that is not the column's, a bit length
//! that disagrees with its bytes) are an `Error` cell.

use std::fmt::Write as _;

use hacivat_values::Value;

use super::{Decoded, bad_length, error};

/// `PGSQL_AF_INET` (`AF_INET`, 2 on every platform PostgreSQL supports).
const AF_INET: u8 = 2;
/// `PGSQL_AF_INET6` (`AF_INET + 1`).
const AF_INET6: u8 = 3;

fn text(s: String) -> Decoded<'static> {
    Decoded::Value(Value::Text(s.into()))
}

/// `inet` (`cidr` false) or `cidr` (`cidr` true).
pub fn inet(raw: &[u8], cidr: bool) -> Decoded<'_> {
    let what = if cidr { "cidr" } else { "inet" };
    let [family, bits, flag, nb, addr @ ..] = raw else {
        return bad_length(what, raw.len(), 8);
    };
    let (max_bits, len) = match *family {
        AF_INET => (32, 4),
        AF_INET6 => (128, 16),
        f => return error("pg-value", format!("an {what} value of address family {f}")),
    };
    if usize::from(*nb) != len || addr.len() != len {
        return error(
            "pg-length",
            format!(
                "an {what} address of {} bytes (header {nb}, expected {len})",
                addr.len()
            ),
        );
    }
    if *bits > max_bits {
        return error("pg-value", format!("an {what} value of {bits} bits"));
    }
    if *flag != u8::from(cidr) {
        return error(
            "pg-value",
            format!("an {what} value whose cidr flag is {flag}"),
        );
    }
    let mut s = String::with_capacity(48);
    if len == 4 {
        push_v4(&mut s, addr);
    } else {
        push_v6(&mut s, addr);
    }
    if cidr || *bits != max_bits {
        let _ = write!(s, "/{bits}");
    }
    text(s)
}

fn push_v4(s: &mut String, addr: &[u8]) {
    for (i, b) in addr.iter().enumerate() {
        if i > 0 {
            s.push('.');
        }
        let _ = write!(s, "{b}");
    }
}

/// `inet_net_ntop_ipv6`'s form of a 16-byte address.
fn push_v6(s: &mut String, addr: &[u8]) {
    let mut words = [0u16; 8];
    for (i, w) in words.iter_mut().enumerate() {
        *w = u16::from_be_bytes([addr[2 * i], addr[2 * i + 1]]);
    }
    // The first longest run of zero words (base, length), if it is at least two long.
    type Run = Option<(usize, usize)>;
    let (mut best, mut cur): (Run, Run) = (None, None);
    for (i, &w) in words.iter().enumerate() {
        if w == 0 {
            cur = Some(cur.map_or((i, 1), |(base, len)| (base, len + 1)));
        } else if let Some(run) = cur.take()
            && best.is_none_or(|b| run.1 > b.1)
        {
            best = Some(run);
        }
    }
    if let Some(run) = cur
        && best.is_none_or(|b| run.1 > b.1)
    {
        best = Some(run);
    }
    let best = best.filter(|&(_, len)| len >= 2);
    for (i, &w) in words.iter().enumerate() {
        if let Some((base, len)) = best
            && (base..base + len).contains(&i)
        {
            if i == base {
                s.push(':');
            }
            continue;
        }
        if i != 0 {
            s.push(':');
        }
        // An embedded IPv4 address: `::a.b.c.d` and `::ffff:a.b.c.d`.
        if i == 6
            && let Some((0, len)) = best
            && (len == 6 || (len == 5 && words[5] == 0xffff))
        {
            push_v4(s, &addr[12..]);
            return;
        }
        let _ = write!(s, "{w:x}");
    }
    if let Some((base, len)) = best
        && base + len == words.len()
    {
        s.push(':');
    }
}

/// `macaddr` (6 bytes) or `macaddr8` (8 bytes).
pub fn macaddr(raw: &[u8], len: usize) -> Decoded<'_> {
    if raw.len() != len {
        let what = if len == 6 { "macaddr" } else { "macaddr8" };
        return bad_length(what, raw.len(), len);
    }
    let mut s = String::with_capacity(3 * len);
    for (i, b) in raw.iter().enumerate() {
        if i > 0 {
            s.push(':');
        }
        let _ = write!(s, "{b:02x}");
    }
    text(s)
}

/// `bit` and `varbit`.
pub fn bits(raw: &[u8]) -> Decoded<'_> {
    let Some((len, data)) = raw.split_first_chunk::<4>() else {
        return bad_length("bit", raw.len(), 4);
    };
    let len = i32::from_be_bytes(*len);
    let Ok(n) = usize::try_from(len) else {
        return error("pg-value", format!("a bit string of length {len}"));
    };
    if data.len() != n.div_ceil(8) {
        return error(
            "pg-length",
            format!("a bit string of {n} bits in {} bytes", data.len()),
        );
    }
    let s = (0..n)
        .map(|i| {
            if data[i / 8] & (0x80 >> (i % 8)) != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    text(s)
}
