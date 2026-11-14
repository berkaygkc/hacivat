//! The block-stack splitter with editor and execution modes (SQL-SPLIT-02; Task 8).

use sql_dialect::{ClientFlavor, DialectCtx};
use sql_lexer::Lexed;
use sql_text::{TextRange, TextSize};

use crate::item::{Confidence, Item, Split, SplitMode, Stmt, StmtHead};
use crate::key::StmtKey;

/// Splits `text` (lexed as `lexed`) in `mode` (Task 8).
///
/// Task 1b stub (fail closed): one `Heuristic` statement over the whole input (none for an
/// empty text), which execution never runs.
pub fn split(
    text: &str,
    lexed: &Lexed,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    mode: SplitMode,
) -> Split {
    let _ = (lexed, ctx, flavor, mode);
    if text.is_empty() {
        return Split::default();
    }
    let len = TextSize::from(u32::try_from(text.len()).expect("a SQL text is under 4 GiB"));
    let all = TextRange::new(TextSize::from(0), len);
    Split {
        items: vec![Item::Statement(Stmt {
            strict: all,
            smart: all,
            terminator: None,
            leading_comments: None,
            confidence: Confidence::Heuristic,
            head: StmtHead::default(),
            key: StmtKey(0),
        })],
        ambiguous: Vec::new(),
        client: Vec::new(),
    }
}
