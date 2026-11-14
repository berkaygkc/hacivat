//! Result values (HV-RES-01), columnar chunks (HV-RES-02 buffers) and cell display formatting
//! (`valfmt`, HV-GRID-10, L3, I18N-11). Core layer; no UI and no `hacivat-l10n` (spec §5.1).
//!
//! Types are complete since M3a Task 1b. Bodies: Task 6 (`decimal`, `temporal`, `columnar`
//! builders), Task 16 (`valfmt`). Working since Task 1b: [`Chunk::from_values`] (a chunk of
//! `BufKind::Values` columns), the [`Chunk`] accessors over it, [`Value::view`] and
//! [`ValueRef::to_value`]. Other bodies panic naming the task that fills them.

pub mod bind;
pub mod columnar;
pub mod decimal;
pub mod logical;
pub mod native;
pub mod special;
pub mod temporal;
pub mod valfmt;
pub mod value;

pub use bind::BindValue;
pub use columnar::{BufKind, CellFlags, Chunk, ChunkBuilder, TempUnit, VarKind};
pub use decimal::{DecimalError, DecimalVal, Repr};
pub use logical::{ColumnMeta, LogicalType, NetKind, ResultSchema};
pub use native::{NativeType, OriginHint, SqliteAffinity};
pub use special::SpecialVal;
pub use temporal::{DateVal, TimeVal, TsVal};
pub use value::{ErrorVal, LenUnit, OpaqueVal, TruncatedVal, Value, ValueRef};
