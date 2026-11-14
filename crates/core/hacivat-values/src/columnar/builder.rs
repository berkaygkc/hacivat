//! `ChunkBuilder` (Task 6): the drivers push decoded cells column by column, row by row, and
//! seal the chunk.
//!
//! Two kinds of trouble, two answers:
//! - **A driver bug panics with a message**: a column index outside the plan, a push that is
//!   not the column's kind (`push_i64` on a `Bool` column; `push_value` fits every column), a
//!   column pushed twice in one row, `end_row` with a column missing, `seal` in the middle of a
//!   row, more than `u32::MAX` rows. The actor's `catch_unwind` turns it into a broken session;
//!   there is no undefined behaviour.
//! - **Data never panics and is never altered**: a value the column's buffer cannot hold
//!   exactly (a `Big` decimal, a timestamp beyond `i64` microseconds, an `f64` that is not an
//!   `f32` in an `f32` column, any [`Value`] of another kind) is kept whole in the column's
//!   exception map; text that is not UTF-8 is kept as `Lossy` bytes; a `Var` cell longer than
//!   its cap is `Truncated` at a character boundary with its total length.
//!
//! A `Var` buffer's offsets are `u32`, so its data never passes `u32::MAX` bytes: a cell that
//! would is kept whole as an exception instead and [`ChunkBuilder::is_full`] turns true, so
//! the driver seals the chunk after that row ("seals early").

use crate::columnar::buf::{Cell, ColumnBuf, Push};
use crate::columnar::{BufKind, Chunk};
use crate::decimal::DecimalVal;
use crate::logical::ResultSchema;
use crate::value::Value;

/// Rows a builder reserves room for at most, whatever `capacity_rows` says.
const MAX_RESERVED_ROWS: usize = 65_536;
/// The bytes a builder reserves up front across its columns at most, counted at 8 a cell
/// (twice the default 4 MiB chunk, which seals first): a 1,600-column result reserves about
/// 650 rows, and the buffers grow past that only if rows arrive.
const MAX_RESERVED_BYTES: usize = 8 << 20;

/// Builds one [`Chunk`] from the driver's plan.
#[derive(Debug)]
pub struct ChunkBuilder {
    plan: Vec<BufKind>,
    columns: Vec<ColumnBuf>,
    rows: u32,
    /// The most bytes one `Var` buffer holds (`u32::MAX`; unit tests lower it).
    var_limit: u64,
    /// A `Var` buffer reached `var_limit`.
    full: bool,
}

impl ChunkBuilder {
    /// A builder for columns of `plan`, sized for `capacity_rows` (a hint: at most 65,536 rows,
    /// and about 8 MiB across the columns, are reserved). Without a schema a `Temporal { unit: Micros }` column holds naive
    /// timestamps (microseconds since 1970-01-01), a `Temporal2` column UTC timestamps with
    /// the offset the server sent, and a `Var(Text)` column text; a time-of-day, `timestamptz`
    /// or XML column needs [`ChunkBuilder::with_schema`].
    pub fn new(plan: &[BufKind], capacity_rows: u32) -> Self {
        Self::build(plan, None, capacity_rows)
    }

    /// A builder whose columns also take their meaning from `schema`'s logical types:
    /// `Temporal { unit: Micros }` holds microseconds of the day for `Time`, UTC microseconds
    /// shown in the display zone for `Timestamp { tz: true }`, naive microseconds otherwise;
    /// `Temporal2` holds a time of day and its offset for `Time`, a UTC timestamp and its
    /// offset otherwise; `Var(Text)` holds XML for `Xml`, text otherwise. Offsets are seconds
    /// east of UTC.
    ///
    /// # Panics
    /// When `plan` and `schema` have different numbers of columns.
    pub fn with_schema(plan: &[BufKind], schema: &ResultSchema, capacity_rows: u32) -> Self {
        assert!(
            plan.len() == schema.columns.len(),
            "a plan of {} columns for a schema of {}",
            plan.len(),
            schema.columns.len()
        );
        Self::build(plan, Some(schema), capacity_rows)
    }

    fn build(plan: &[BufKind], schema: Option<&ResultSchema>, capacity_rows: u32) -> Self {
        let rows = reserved_rows(plan.len(), capacity_rows);
        let columns = plan
            .iter()
            .enumerate()
            .map(|(i, &kind)| {
                let logical = schema.map(|s| &s.columns[i].logical);
                ColumnBuf::new(kind, logical, rows)
            })
            .collect();
        ChunkBuilder {
            plan: plan.to_vec(),
            columns,
            rows: 0,
            var_limit: u64::from(u32::MAX),
            full: false,
        }
    }

    /// SQL NULL in any column.
    pub fn push_null(&mut self, col: u32) {
        self.column(col).push_null();
    }

    pub fn push_bool(&mut self, col: u32, v: bool) {
        self.typed(col, "push_bool", Cell::Bool(v));
    }

    pub fn push_i64(&mut self, col: u32, v: i64) {
        self.typed(col, "push_i64", Cell::I64(v));
    }

    pub fn push_u64(&mut self, col: u32, v: u64) {
        self.typed(col, "push_u64", Cell::U64(v));
    }

    /// An `F64` cell; in an `f32` column a value that is not exactly an `f32` is kept as an
    /// `F64` exception (the driver passes an `f32` widened, which is exact).
    pub fn push_f64(&mut self, col: u32, v: f64) {
        self.typed(col, "push_f64", Cell::F64(v));
    }

    /// A `Dec64` cell: the mantissa at the column's scale.
    pub fn push_dec64(&mut self, col: u32, mant: i64) {
        self.typed(col, "push_dec64", Cell::Dec64(mant));
    }

    /// A `Dec64` or `Dec128` cell. Kept exactly as an exception instead: in a `Dec64` column a
    /// value of another scale, beyond an `i64` mantissa, or `-0`; in either, more than 38
    /// digits.
    pub fn push_decimal(&mut self, col: u32, v: &DecimalVal) {
        self.typed(col, "push_decimal", Cell::Decimal(v));
    }

    /// A `Temporal` cell: days since 1970-01-01 (`Days`), or microseconds (`Micros`: of the
    /// day, or since 1970-01-01, see [`ChunkBuilder::with_schema`]). A day outside `i32` or a
    /// time of day outside `0 ..= 24:00:00` is an `Error` exception.
    pub fn push_temporal(&mut self, col: u32, a: i64) {
        self.typed(col, "push_temporal", Cell::Temporal(a));
    }

    /// A `Temporal2` cell: microseconds (of the day, or since 1970-01-01 UTC) and an offset
    /// in seconds east of UTC.
    pub fn push_temporal2(&mut self, col: u32, a: i64, b: i32) {
        self.typed(col, "push_temporal2", Cell::Temporal2(a, b));
    }

    /// A `Var` cell: truncated at `cap` bytes (at a UTF-8 boundary for text); text that is not
    /// valid UTF-8 becomes `Lossy`.
    pub fn push_bytes(&mut self, col: u32, v: &[u8], cap: u32) {
        self.typed(col, "push_bytes", Cell::Bytes(v, cap));
    }

    pub fn push_fixed16(&mut self, col: u32, v: [u8; 16]) {
        self.typed(col, "push_fixed16", Cell::Fixed16(v));
    }

    /// A whole value (a `Values` column, or an exception in a typed one). In a typed column a
    /// value the buffer holds exactly (`Value::Int` in an `I64` column, `Value::Text` in a
    /// `Var(Text)` one) is stored inline; anything else is an exception. Never truncates.
    pub fn push_value(&mut self, col: u32, v: Value) {
        let limit = self.var_limit;
        if self.column(col).push_value(v, limit) == Push::Full {
            self.full = true;
        }
    }

    /// Ends the row.
    ///
    /// # Panics
    /// When a column has no cell in this row, or the chunk already has `u32::MAX` rows.
    pub fn end_row(&mut self) {
        let next = self.rows as usize + 1;
        if let Some(col) = self.columns.iter().position(|c| c.len() != next) {
            panic!("column {col} has no cell in row {}", self.rows);
        }
        self.rows = self
            .rows
            .checked_add(1)
            .expect("a chunk holds at most u32::MAX rows");
    }

    /// The rows ended so far.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The bytes the chunk would hold if sealed now (the memory budget's measure; the driver
    /// seals at `chunk_bytes`).
    pub fn bytes(&self) -> u64 {
        self.columns.iter().map(ColumnBuf::bytes).sum()
    }

    /// Whether the chunk must be sealed after this row: a `Var` buffer reached `u32::MAX`
    /// bytes, or the chunk has `u32::MAX` rows.
    pub fn is_full(&self) -> bool {
        self.full || self.rows == u32::MAX
    }

    /// The chunk, its buffers shrunk to fit.
    ///
    /// # Panics
    /// In the middle of a row (a cell pushed after the last `end_row`).
    pub fn seal(mut self) -> Chunk {
        if let Some(col) = self
            .columns
            .iter()
            .position(|c| c.len() != self.rows as usize)
        {
            panic!(
                "seal in the middle of row {}: column {col} has a cell",
                self.rows
            );
        }
        for c in &mut self.columns {
            c.shrink_to_fit();
        }
        let bytes = self.bytes();
        Chunk {
            rows: self.rows,
            columns: self.columns,
            bytes,
        }
    }

    /// Column `col`, which has no cell in this row yet.
    fn column(&mut self, col: u32) -> &mut ColumnBuf {
        let width = self.columns.len();
        let row = self.rows;
        let c = self
            .columns
            .get_mut(col as usize)
            .unwrap_or_else(|| panic!("column {col} outside plan of {width}"));
        assert!(
            c.len() == row as usize,
            "column {col} pushed twice in row {row}"
        );
        c
    }

    fn typed(&mut self, col: u32, what: &str, cell: Cell<'_>) {
        let limit = self.var_limit;
        match self.column(col).push(cell, limit) {
            Push::Done => {}
            Push::Full => self.full = true,
            Push::Mismatch => panic!(
                "{what} on column {col}, a {:?} column",
                self.plan[col as usize]
            ),
        }
    }
}

/// The rows reserved up front for `width` columns asked to hold `capacity_rows`.
fn reserved_rows(width: usize, capacity_rows: u32) -> usize {
    let per_row = width.max(1) * size_of::<u64>();
    (capacity_rows as usize)
        .min(MAX_RESERVED_ROWS)
        .min(MAX_RESERVED_BYTES / per_row)
}

#[cfg(test)]
mod tests {
    use super::{ChunkBuilder, reserved_rows};

    #[test]
    fn the_up_front_reservation_is_bounded() {
        assert_eq!(reserved_rows(50, 4_096), 4_096);
        assert_eq!(reserved_rows(1, u32::MAX), 65_536);
        assert_eq!(reserved_rows(1_600, 4_096), (8 << 20) / (1_600 * 8));
        assert_eq!(reserved_rows(0, 10), 10);
    }
    use crate::columnar::{BufKind, VarKind};
    use crate::value::{Value, ValueRef};

    #[test]
    fn a_var_buffer_at_its_limit_keeps_the_cell_whole_and_asks_to_seal() {
        let mut b = ChunkBuilder::new(&[BufKind::Var(VarKind::Text)], 4);
        b.var_limit = 10;
        b.push_bytes(0, b"0123456789", 1 << 20);
        b.end_row();
        assert!(!b.is_full(), "exactly at the limit still fits");
        b.push_bytes(0, b"abc", 1 << 20);
        b.end_row();
        assert!(b.is_full());
        // A whole value past the limit is an exception too.
        b.push_value(0, Value::Text("defg".into()));
        b.end_row();
        let c = b.seal();
        assert!(matches!(c.cell(0, 0), ValueRef::Text("0123456789")));
        assert!(matches!(c.cell(1, 0), ValueRef::Text("abc")));
        assert!(matches!(c.cell(2, 0), ValueRef::Text("defg")));
        assert!(c.flags(1, 0).is_empty());
    }

    #[test]
    fn a_bytes_buffer_at_its_limit_keeps_the_cell_whole() {
        let mut b = ChunkBuilder::new(&[BufKind::Var(VarKind::Bytes)], 1);
        b.var_limit = 2;
        b.push_bytes(0, &[1, 2, 3], 8);
        b.end_row();
        assert!(b.is_full());
        assert!(matches!(b.seal().cell(0, 0), ValueRef::Bytes([1, 2, 3])));
    }
}
