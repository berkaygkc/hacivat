//! Parsers for the PostgreSQL text output forms the catalog queries return: one-dimensional
//! arrays (`{a,"b c",NULL}`), `int2vector`/`oidvector` (`1 2 3`) and booleans (`t`/`f`).
//!
//! The simple query protocol returns every value as text, so aggregated catalog values
//! (column lists, enum labels, `reloptions`) arrive as array literals.

/// A one-dimensional array literal's elements; `None` is an unquoted `NULL`. `None` overall
/// when `s` is not a well-formed one-dimensional array literal.
pub fn parse_array(s: &str) -> Option<Vec<Option<String>>> {
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    let mut out = Vec::new();
    if inner.is_empty() {
        return Some(out);
    }
    let mut chars = inner.chars().peekable();
    loop {
        if chars.peek() == Some(&'"') {
            chars.next();
            let mut v = String::new();
            loop {
                match chars.next()? {
                    '\\' => v.push(chars.next()?),
                    '"' => break,
                    c => v.push(c),
                }
            }
            out.push(Some(v));
        } else {
            let mut v = String::new();
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                if c == '{' || c == '}' || c == '"' {
                    return None;
                }
                v.push(c);
                chars.next();
            }
            if v.is_empty() {
                return None;
            }
            out.push(if v == "NULL" { None } else { Some(v) });
        }
        match chars.next() {
            None => return Some(out),
            Some(',') => {}
            Some(_) => return None,
        }
    }
}

/// An array literal of numbers (`{1,2}` from `int2[]`/`oid[]`); NULL elements are an error.
pub fn parse_num_array<T: std::str::FromStr>(s: &str) -> Option<Vec<T>> {
    parse_array(s)?
        .into_iter()
        .map(|e| e.and_then(|v| v.parse().ok()))
        .collect()
}

/// `int2vector`/`oidvector` text: space-separated numbers (`1 2 0`); empty is no elements.
pub fn parse_vector<T: std::str::FromStr>(s: &str) -> Option<Vec<T>> {
    s.split_ascii_whitespace().map(|v| v.parse().ok()).collect()
}

/// `t`/`f`.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "t" => Some(true),
        "f" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn arrays_parse_quoting_escapes_and_nulls() {
        assert_eq!(parse_array("{}"), Some(vec![]));
        assert_eq!(parse_array("{a,b}"), Some(vec![s("a"), s("b")]));
        assert_eq!(
            parse_array(r#"{"a b","c\"d","e\\f",NULL,"NULL"}"#),
            Some(vec![s("a b"), s("c\"d"), s("e\\f"), None, s("NULL")])
        );
        assert_eq!(
            parse_array("{fillfactor=70,autovacuum_enabled=false}"),
            Some(vec![s("fillfactor=70"), s("autovacuum_enabled=false")])
        );
        assert_eq!(parse_array(r#"{"",ş}"#), Some(vec![s(""), s("ş")]));
        assert_eq!(
            parse_array(r#"{NULL,"(a + b)"}"#),
            Some(vec![None, s("(a + b)")])
        );
    }

    #[test]
    fn malformed_arrays_are_refused() {
        for bad in [
            "",
            "{",
            "a,b",
            "{a,}",
            "{,a}",
            r#"{"a}"#,
            "{{1,2},{3,4}}",
            r#"{"a"b}"#,
        ] {
            assert_eq!(parse_array(bad), None, "{bad}");
        }
    }

    #[test]
    fn numbers_and_vectors() {
        assert_eq!(parse_num_array::<i16>("{1,2,-3}"), Some(vec![1, 2, -3]));
        assert_eq!(parse_num_array::<u32>("{}"), Some(vec![]));
        assert_eq!(parse_num_array::<i16>("{1,NULL}"), None);
        assert_eq!(parse_vector::<i16>("1 2 0"), Some(vec![1, 2, 0]));
        assert_eq!(parse_vector::<i16>(""), Some(vec![]));
        assert_eq!(parse_vector::<i16>("1 x"), None);
        assert_eq!(parse_bool("t"), Some(true));
        assert_eq!(parse_bool("f"), Some(false));
        assert_eq!(parse_bool("true"), None);
    }
}
