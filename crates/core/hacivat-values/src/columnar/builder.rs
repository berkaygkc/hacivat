//! `ChunkBuilder` (Task 6): the drivers push decoded cells column by column, row by row, and
//! seal the chunk. A column index outside the plan panics with a message (the actor's
//! `catch_unwind` turns a driver bug into a broken session; no undefined behaviour).
//!
//! Task 1b stub: `new` records the plan; every other method panics naming Task 6.

use crate::columnar::{BufKind, Chunk};
use crate::decimal::DecimalVal;
use crate::value::Value;

/// Builds one [`Chunk`] from the driver's plan.
#[derive(Debug)]
pub struct ChunkBuilder {
    plan: Vec<BufKind>,
    capacity_rows: u32,
}

impl ChunkBuilder {
    /// A builder for columns of `plan`, sized for `capacity_rows`.
    pub fn new(plan: &[BufKind], capacity_rows: u32) -> Self {
        ChunkBuilder {
            plan: plan.to_vec(),
            capacity_rows,
        }
    }

    fn todo(&self, what: &str) -> ! {
        unimplemented!(
            "ChunkBuilder::{what} (plan of {}, {} rows): Task 6",
            self.plan.len(),
            self.capacity_rows
        )
    }

    pub fn push_null(&mut self, col: u32) {
        let _ = col;
        self.todo("push_null")
    }

    pub fn push_bool(&mut self, col: u32, v: bool) {
        let _ = (col, v);
        self.todo("push_bool")
    }

    pub fn push_i64(&mut self, col: u32, v: i64) {
        let _ = (col, v);
        self.todo("push_i64")
    }

    pub fn push_u64(&mut self, col: u32, v: u64) {
        let _ = (col, v);
        self.todo("push_u64")
    }

    pub fn push_f64(&mut self, col: u32, v: f64) {
        let _ = (col, v);
        self.todo("push_f64")
    }

    /// A `Dec64` cell: the mantissa at the column's scale.
    pub fn push_dec64(&mut self, col: u32, mant: i64) {
        let _ = (col, mant);
        self.todo("push_dec64")
    }

    pub fn push_decimal(&mut self, col: u32, v: &DecimalVal) {
        let _ = (col, v);
        self.todo("push_decimal")
    }

    pub fn push_temporal(&mut self, col: u32, a: i64) {
        let _ = (col, a);
        self.todo("push_temporal")
    }

    pub fn push_temporal2(&mut self, col: u32, a: i64, b: i32) {
        let _ = (col, a, b);
        self.todo("push_temporal2")
    }

    /// A `Var` cell: truncated at `cap` bytes (at a UTF-8 boundary for text); text that is not
    /// valid UTF-8 becomes `Lossy`.
    pub fn push_bytes(&mut self, col: u32, v: &[u8], cap: u32) {
        let _ = (col, v, cap);
        self.todo("push_bytes")
    }

    pub fn push_fixed16(&mut self, col: u32, v: [u8; 16]) {
        let _ = (col, v);
        self.todo("push_fixed16")
    }

    /// A whole value (a `Values` column, or an exception in a typed one).
    pub fn push_value(&mut self, col: u32, v: Value) {
        let _ = (col, v);
        self.todo("push_value")
    }

    pub fn end_row(&mut self) {
        self.todo("end_row")
    }

    pub fn rows(&self) -> u32 {
        self.todo("rows")
    }

    pub fn bytes(&self) -> u64 {
        self.todo("bytes")
    }

    pub fn seal(self) -> Chunk {
        self.todo("seal")
    }
}
