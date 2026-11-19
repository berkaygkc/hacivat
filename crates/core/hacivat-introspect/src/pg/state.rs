// ## m4-t16-pg
//! `StateOf<Kind>` stamps (HV-INTRO-15; R-M4-48; `packs/pg/30-state.hvq.sql`): the raw state
//! marks of objects for `check_state`. Each section returns, per object oid, the xmins of its
//! own row, its comment rows, its children's rows and the rows one `pg_depend` hop away; the
//! mark is an XXH3-64 of the sorted rows, so a comment removal, a second trigger or a renamed
//! dependency moves it. Task 22 compares marks with the snapshot's by equality (its fenced
//! `// ## m4-t22-state` blocks go in this file).

use std::collections::{BTreeMap, BTreeSet};

use hacivat_meta::{Level, Name, ObjectKey, ObjectKind, ServerId, StateMark};
use hacivat_session::meta::MetaConn;
use tokio_util::sync::CancellationToken;

use super::decode::IdentityRow;
use super::model::Oid;
use super::pass::{PassHooks, PassReport, in_snapshot};
use super::sections::{Binds, Reader, name};
use crate::introspector::IntroError;
use crate::pack::Mode;

/// The state section of a key's kind (`None`: the kind has no PostgreSQL oid of its own).
pub fn state_section(kind: ObjectKind) -> Option<&'static str> {
    use ObjectKind as K;
    Some(match kind {
        K::Table | K::View | K::MaterializedView | K::ForeignTable | K::Sequence | K::Index => {
            name::STATE_OF_RELATION
        }
        K::Routine | K::Procedure | K::Aggregate => name::STATE_OF_ROUTINE,
        K::Domain | K::EnumType | K::CompositeType | K::RangeType | K::Unknown => {
            name::STATE_OF_TYPE
        }
        K::PrimaryKey | K::UniqueKey | K::ForeignKey | K::Check | K::Exclusion => {
            name::STATE_OF_CONSTRAINT
        }
        K::Trigger => name::STATE_OF_TRIGGER,
        _ => return None,
    })
}

/// The raw state marks of `keys`: `Some(StateMark::Hash)` per object, `None` when its own row
/// is gone. A key without a PostgreSQL oid (a column) has no mark of its own:
/// `Some(StateMark::None)`.
pub async fn stamps(
    c: &mut MetaConn,
    keys: &[ObjectKey],
    cancel: &CancellationToken,
) -> Result<(Vec<Option<StateMark>>, PassReport), IntroError> {
    let mut report = PassReport::default();
    let mut by_section: BTreeMap<&'static str, BTreeSet<Oid>> = BTreeMap::new();
    for k in keys {
        if let (Some(s), Some(ServerId::Oid(o))) = (state_section(k.kind), k.server_id) {
            by_section.entry(s).or_default().insert(o);
        }
    }
    let rows = in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, _id: &IdentityRow| {
            let mut out: BTreeMap<(&'static str, Oid), Vec<(String, String)>> = BTreeMap::new();
            for (&section, oids) in &by_section {
                let binds = Binds::default().with_oids(oids.iter().copied().collect());
                for (o, src, stamp) in r
                    .read(section, Mode::Full, Level::Structure, &binds, |row| {
                        Ok((row.num::<Oid>(0)?, row.string(1)?, row.string(2)?))
                    })
                    .await?
                {
                    out.entry((section, o)).or_default().push((src, stamp));
                }
            }
            Ok(out)
        },
    )
    .await?;
    let marks = keys
        .iter()
        .map(|k| match (state_section(k.kind), k.server_id) {
            (Some(s), Some(ServerId::Oid(o))) => {
                let rs = rows.get(&(s, o))?;
                if !rs.iter().any(|(src, _)| src == "self") {
                    return None;
                }
                Some(StateMark::Hash(stamp_hash(rs)))
            }
            _ => Some(StateMark::None),
        })
        .collect();
    Ok((marks, report))
}

/// XXH3-64 of the sorted `(src, stamp)` rows.
pub fn stamp_hash(rows: &[(String, String)]) -> u64 {
    let mut v: Vec<&(String, String)> = rows.iter().collect();
    v.sort();
    let mut text = String::new();
    for (s, x) in v {
        text.push_str(s);
        text.push('\u{1f}');
        text.push_str(x);
        text.push('\u{1e}');
    }
    twox_hash::XxHash3_64::oneshot(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_hash_is_order_free_and_moves_with_any_row() {
        let a = vec![
            ("self".to_string(), "10".to_string()),
            ("desc".into(), "0:11".into()),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(stamp_hash(&a), stamp_hash(&b));
        assert_ne!(stamp_hash(&a), stamp_hash(&a[..1]));
    }
}
