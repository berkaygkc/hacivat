//! The execution-range policies (D-011; Task 13).

/// What a caret inside a statement runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsidePolicy {
    /// Offer the candidates (one candidate runs directly).
    Ask,
    SmallestSubqueryOrStatement,
    SmallestStatement,
    LargestStatement,
    LargestStatementOrBatch,
    WholeScript,
    EverythingFromCaret,
}

/// What a caret outside every statement runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutsidePolicy {
    Nothing,
    WholeScript,
    EverythingBelow,
}

/// How a selection runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionPolicy {
    AsOne,
    AsSeparate,
    SmartExpand,
}

/// A Run action's policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecPolicy {
    pub inside: InsidePolicy,
    pub outside: OutsidePolicy,
    pub selection: SelectionPolicy,
}

impl Default for ExecPolicy {
    /// D-011: Ask (with the one-candidate shortcut), Nothing outside a statement, a selection
    /// as separate statements.
    fn default() -> Self {
        ExecPolicy {
            inside: InsidePolicy::Ask,
            outside: OutsidePolicy::Nothing,
            selection: SelectionPolicy::AsSeparate,
        }
    }
}

impl ExecPolicy {
    /// Run 2: the largest statement at the caret.
    pub fn largest() -> Self {
        ExecPolicy {
            inside: InsidePolicy::LargestStatement,
            outside: OutsidePolicy::Nothing,
            selection: SelectionPolicy::AsSeparate,
        }
    }

    /// Run 3: the whole script.
    pub fn whole_script() -> Self {
        ExecPolicy {
            inside: InsidePolicy::WholeScript,
            outside: OutsidePolicy::WholeScript,
            selection: SelectionPolicy::AsSeparate,
        }
    }
}
