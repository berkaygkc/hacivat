//! The streaming splitter (SQL-SPLIT-05; Task 5): statements out of a script fed in chunks
//! (file execution), with the lexer state and the prepass state carried across chunks.
//!
//! STUB until Task 5 (fail closed): [`StreamSplitter`] yields nothing while fed and one
//! `Ambiguous { offset: 0, reason: UnknownBlock }` at [`StreamSplitter::finish`], so nothing
//! is ever run from a stream.

use sql_dialect::{ClientFlavor, DialectCtx};

use crate::item::{AmbiguityReason, Confidence};

/// The longest statement the streaming splitter holds (64 MiB); a longer one is
/// [`StreamItem::TooLong`].
pub const STREAM_STATEMENT_MAX: usize = 64 << 20;

/// One item of a stream; offsets are byte offsets from the start of the stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StreamItem {
    Statement {
        text: String,
        offset: u64,
        confidence: Confidence,
    },
    Client {
        text: String,
        offset: u64,
        supported: bool,
    },
    Ambiguous {
        offset: u64,
        reason: AmbiguityReason,
    },
    TooLong {
        offset: u64,
    },
}

/// Splits a script fed in chunks. STUB: see the module documentation.
#[derive(Debug)]
pub struct StreamSplitter {
    _stub: (),
}

impl StreamSplitter {
    pub fn new(ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        let _ = (ctx, flavor);
        StreamSplitter { _stub: () }
    }

    /// The items `chunk` completes. STUB: none.
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamItem> {
        let _ = chunk;
        Vec::new()
    }

    /// The remaining items at the end of the stream. STUB: one `Ambiguous` at offset 0.
    pub fn finish(self) -> Vec<StreamItem> {
        vec![StreamItem::Ambiguous {
            offset: 0,
            reason: AmbiguityReason::UnknownBlock,
        }]
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::DialectId;

    use super::*;

    #[test]
    fn stub_runs_nothing() {
        let mut s = StreamSplitter::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::Psql,
        );
        assert!(s.feed("SELECT 1;\nSELECT 2;\n").is_empty());
        assert_eq!(
            s.finish(),
            [StreamItem::Ambiguous {
                offset: 0,
                reason: AmbiguityReason::UnknownBlock
            }]
        );
    }
}
