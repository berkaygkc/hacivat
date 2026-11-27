//! The classifier's input limits (R-M3-3; Task 4 enforces them in `classify` step 1, before
//! any parser runs).

use sql_oracle_pg::MAX_ORACLE_NESTING;

/// Input limits; past one, the request is `Unknown` with `TooComplex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// 1 MiB.
    pub max_bytes: usize,
    /// 200,000.
    pub max_tokens: u32,
    /// 256, the oracle's own bracket limit (`MAX_ORACLE_NESTING`).
    pub max_nesting: u16,
    /// 16 executable-comment readings.
    pub max_exec_comment_views: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 1 << 20,
            max_tokens: 200_000,
            max_nesting: MAX_ORACLE_NESTING,
            max_exec_comment_views: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_plans() {
        let l = Limits::default();
        assert_eq!(
            (
                l.max_bytes,
                l.max_tokens,
                l.max_nesting,
                l.max_exec_comment_views
            ),
            (1_048_576, 200_000, 256, 16)
        );
    }
}
