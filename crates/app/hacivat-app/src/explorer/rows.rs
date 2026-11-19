//! Explorer rows (Task 14): what each node shows — the kind icon, the verbatim, bidi-isolated
//! name, type text at `text-2` for columns, kind-group headers with counts in the format
//! locale, and the "Built-in" group last and collapsed (mock 1).
//!
//! - [`DisplayGroup`]: the kind groups a container's children are shown in, in a fixed order
//!   (NOTES M4 surface 1): Tables, Views, Materialized views, Functions, Procedures, Sequences,
//!   Types, Events under a schema; Columns, Keys, Foreign keys, Indexes, Checks, Triggers under
//!   an object. A kind with no group (an extension, a partition, an `Unknown` object) is listed
//!   after the groups, never hidden.
//! - [`kind_icon`]: the 12 px kind icon of a row (the mock's `KIND_IC`).
//! - [`NameOrder`]: display order of names by the ICU4X collation of the UI language
//!   (cross-cutting #4: `users` before `Users`, `ı` before `i` in Turkish), with a byte-order
//!   tie-break so the order is total. Display only: identity, keys and DDL order never use it.
//!   Columns keep their ordinal order (NOTES ruling 17).
//! - [`isolate`]: a catalog name between FSI and PDI (HV-MODEL-01 i18n notes), the way Fluent
//!   isolates a placeable. The text is the stored one, never case-mapped or translated.
//! - [`type_text`]: the mono text after a name (a column's raw type, a key's or index's
//!   columns, a foreign key's target, an `Unknown` object's raw server kind).

use std::cmp::Ordering;

use hacivat_meta::props::Props;
use hacivat_meta::{Obj, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_ui::icon::IconName;
use hacivat_ui_core::gpui_surface::SharedString;
use hacivat_values::LogicalType;
use hacivat_values::ValueRef;
use hacivat_values::valfmt::compare::{Comparator, comparator_for};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts};

/// A kind group of the explorer: a small-caps header with a count (R-M4-49: it takes the
/// cursor, never the selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayGroup {
    Tables,
    Views,
    MaterializedViews,
    Functions,
    Procedures,
    Sequences,
    Types,
    Events,
    Columns,
    Keys,
    ForeignKeys,
    Indexes,
    Checks,
    Triggers,
}

impl DisplayGroup {
    /// The fixed order of the groups under one container.
    pub const ORDER: [DisplayGroup; 14] = [
        DisplayGroup::Tables,
        DisplayGroup::Views,
        DisplayGroup::MaterializedViews,
        DisplayGroup::Functions,
        DisplayGroup::Procedures,
        DisplayGroup::Sequences,
        DisplayGroup::Types,
        DisplayGroup::Events,
        DisplayGroup::Columns,
        DisplayGroup::Keys,
        DisplayGroup::ForeignKeys,
        DisplayGroup::Indexes,
        DisplayGroup::Checks,
        DisplayGroup::Triggers,
    ];

    /// The group objects of `kind` are shown in, or `None` (listed after the groups).
    pub fn of(kind: ObjectKind) -> Option<DisplayGroup> {
        use ObjectKind::*;
        Some(match kind {
            Table | ForeignTable | VirtualTable => DisplayGroup::Tables,
            View => DisplayGroup::Views,
            MaterializedView => DisplayGroup::MaterializedViews,
            Routine | Aggregate => DisplayGroup::Functions,
            Procedure => DisplayGroup::Procedures,
            Sequence => DisplayGroup::Sequences,
            Domain | EnumType | CompositeType | RangeType | AliasType | TableType | ObjectType => {
                DisplayGroup::Types
            }
            Event => DisplayGroup::Events,
            Column => DisplayGroup::Columns,
            PrimaryKey | UniqueKey => DisplayGroup::Keys,
            ForeignKey => DisplayGroup::ForeignKeys,
            Index => DisplayGroup::Indexes,
            Check | Exclusion => DisplayGroup::Checks,
            Trigger => DisplayGroup::Triggers,
            _ => return None,
        })
    }

    /// A stable code (part of the group header's `TreeKey`).
    pub fn code(self) -> u8 {
        match self {
            DisplayGroup::Tables => 1,
            DisplayGroup::Views => 2,
            DisplayGroup::MaterializedViews => 3,
            DisplayGroup::Functions => 4,
            DisplayGroup::Procedures => 5,
            DisplayGroup::Sequences => 6,
            DisplayGroup::Types => 7,
            DisplayGroup::Events => 8,
            DisplayGroup::Columns => 9,
            DisplayGroup::Keys => 10,
            DisplayGroup::ForeignKeys => 11,
            DisplayGroup::Indexes => 12,
            DisplayGroup::Checks => 13,
            DisplayGroup::Triggers => 14,
        }
    }

    /// The header's Fluent id (small caps, pre-cased in the file).
    pub fn message_id(self) -> &'static str {
        match self {
            DisplayGroup::Tables => "hv-explorer-kind-tables",
            DisplayGroup::Views => "hv-explorer-kind-views",
            DisplayGroup::MaterializedViews => "hv-explorer-kind-matviews",
            DisplayGroup::Functions => "hv-explorer-kind-functions",
            DisplayGroup::Procedures => "hv-explorer-kind-procedures",
            DisplayGroup::Sequences => "hv-explorer-kind-sequences",
            DisplayGroup::Types => "hv-explorer-kind-types",
            DisplayGroup::Events => "hv-explorer-kind-events",
            DisplayGroup::Columns => "hv-explorer-kind-columns",
            DisplayGroup::Keys => "hv-explorer-kind-keys",
            DisplayGroup::ForeignKeys => "hv-explorer-kind-foreign-keys",
            DisplayGroup::Indexes => "hv-explorer-kind-indexes",
            DisplayGroup::Checks => "hv-explorer-kind-checks",
            DisplayGroup::Triggers => "hv-explorer-kind-triggers",
        }
    }

    /// Columns keep the server's ordinal order (NOTES ruling 17); every other group sorts by
    /// the UI collation.
    pub fn keeps_ordinal_order(self) -> bool {
        self == DisplayGroup::Columns
    }
}

/// The 12 px icon of an object of `kind` (NOTES M4 surface 1).
pub fn kind_icon(kind: ObjectKind) -> IconName {
    use ObjectKind::*;
    match kind {
        Database => IconName::Cylinder,
        Schema => IconName::FolderTree,
        Table | ForeignTable | VirtualTable | Partition | Subpartition => IconName::Table2,
        View => IconName::Eye,
        MaterializedView => IconName::TableProperties,
        Column => IconName::Columns3,
        PrimaryKey | UniqueKey => IconName::KeyRound,
        ForeignKey => IconName::Link,
        Index => IconName::ListTree,
        Check | Exclusion => IconName::ShieldCheck,
        Trigger | EventTrigger => IconName::Zap,
        Routine | Procedure | Aggregate => IconName::SquareFunction,
        Sequence => IconName::ListOrdered,
        Domain | EnumType | CompositeType | RangeType | AliasType | TableType | ObjectType => {
            IconName::Type
        }
        Event => IconName::Clock,
        Role | User | Login => IconName::Users,
        _ => IconName::Square,
    }
}

/// `name` between FSI (U+2068) and PDI (U+2069): verbatim, isolated from the text around it.
pub fn isolate(name: &str) -> SharedString {
    let mut s = String::with_capacity(name.len() + 6);
    s.push('\u{2068}');
    s.push_str(name);
    s.push('\u{2069}');
    s.into()
}

/// Display order of names: the ICU4X collator of the UI language (tertiary strength), ties
/// broken by byte order.
pub struct NameOrder {
    lang: String,
    cmp: Comparator,
}

impl std::fmt::Debug for NameOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameOrder")
            .field("lang", &self.lang)
            .finish_non_exhaustive()
    }
}

impl NameOrder {
    /// The order of the UI language `lang` (a BCP 47 tag, `en`, `tr-TR`); the root collation
    /// for an empty or unknown tag.
    pub fn new(lang: &str) -> NameOrder {
        let tag = if lang.trim().is_empty() { "und" } else { lang };
        let text = LogicalType::Text {
            max_chars: None,
            fixed: false,
        };
        // The collation of the UI language, written the way PostgreSQL names an ICU collation
        // (`tr-x-icu`): `hacivat-values` resolves it to the ICU4X collator of that locale.
        let facts = ColumnFacts {
            logical: &text,
            native_name: "",
            id_like: false,
            binary: BinaryDisplay::default(),
            engine: DbmsKind::Postgres,
        };
        let collation = format!("{tag}-x-icu");
        NameOrder {
            lang: tag.to_string(),
            cmp: comparator_for(&facts, Some(&collation)),
        }
    }

    /// The language the order was made for.
    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// `a` against `b` for display.
    pub fn cmp(&self, a: &str, b: &str) -> Ordering {
        self.cmp
            .cmp(ValueRef::Text(a), ValueRef::Text(b))
            .then_with(|| a.cmp(b))
    }
}

/// The mono text after an object's name, if it has one (a rendered value, never a Fluent
/// sentence: types, column lists and kinds are the server's own text).
pub fn type_text(obj: &Obj) -> Option<SharedString> {
    if obj.kind() == ObjectKind::Unknown {
        return obj.raw_kind().map(|k| isolate(k));
    }
    match obj.props() {
        Props::Column(c) if !c.raw_type.is_empty() => Some(isolate(&c.raw_type)),
        Props::Key(k) => column_list(k.columns.iter().filter_map(|c| c.name.as_deref())),
        Props::Index(i) => column_list(i.key.columns.iter().filter_map(|c| c.name.as_deref())),
        Props::ForeignKey(fk) => {
            let target = fk.target.path.last()?;
            let cols = fk
                .target_columns
                .iter()
                .map(|c| c.as_ref())
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("→ \u{2068}{target}\u{2069} (\u{2068}{cols}\u{2069})").into())
        }
        _ => None,
    }
}

/// `(a, b)`: a key's or an index's columns, verbatim and isolated.
fn column_list<'a>(names: impl Iterator<Item = &'a str>) -> Option<SharedString> {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        return None;
    }
    Some(format!("(\u{2068}{}\u{2069})", names.join(", ")).into())
}

/// Sorts `members` (arena indexes of `objs`) for display: by ordinal for columns, else by the
/// name order.
pub fn sort_members(
    members: &mut [u32],
    objs: &[Obj],
    group: Option<DisplayGroup>,
    order: &NameOrder,
) {
    if group.is_some_and(DisplayGroup::keeps_ordinal_order) {
        let ordinal = |i: u32| match objs[i as usize].props() {
            Props::Column(c) => c.ordinal,
            _ => u32::MAX,
        };
        // Stable: equal ordinals keep arena (insertion) order.
        members.sort_by_key(|&i| ordinal(i));
        return;
    }
    members.sort_by(|&a, &b| order.cmp(objs[a as usize].name(), objs[b as usize].name()));
}
