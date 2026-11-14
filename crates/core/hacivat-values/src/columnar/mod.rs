//! Columnar chunks (HV-RES-02 buffers; Task 6): one buffer per column, a validity bitmap,
//! exceptions in a sparse map, and `cell` returning a [`ValueRef`] without allocating.
//!
//! The drivers build chunks with [`ChunkBuilder`] (one typed buffer per [`BufKind`]);
//! [`Chunk::from_values`] (Task 1b) builds one of [`BufKind::Values`] buffers for fixtures.
//!
//! Bytes per cell, besides one validity bit: `Bool` 1/8; `I64`, `U64`, `F64`, `Temporal
//! { Micros }` 8; `F64 { f32 }`, `Temporal { Days }` 4; `Temporal2` 12; `Fixed16` 16; `Dec64`
//! and `Dec128` 24 (a whole `DecimalVal`, because `ValueRef::Decimal` borrows one); `Var` 4 plus
//! the payload; `Values` 32 plus the payload; an exception 36 plus its payload.

mod bitmap;
mod buf;
pub mod builder;

pub use builder::ChunkBuilder;

use crate::logical::ResultSchema;
use crate::value::{Value, ValueRef};
use buf::ColumnBuf;

bitflags::bitflags! {
    /// What a cell carries besides its value.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct CellFlags: u8 {
        const TRUNCATED = 1;
        const LOSSY = 2;
        const ERROR = 4;
        const SPECIAL = 8;
    }
}

/// The unit of a temporal buffer's `i64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TempUnit {
    Days,
    Micros,
}

/// What a variable-length buffer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarKind {
    Text,
    Bytes,
    Json,
}

/// The buffer kind of one column (the driver's plan for a result set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufKind {
    Bool,
    I64,
    U64,
    F64 {
        f32: bool,
    },
    Dec64 {
        scale: i16,
    },
    Dec128,
    Temporal {
        unit: TempUnit,
    },
    /// An `i64` and an `i32` (a timestamp with its offset, a time with its zone).
    Temporal2,
    Var(VarKind),
    Fixed16,
    /// Whole [`Value`]s (fixtures, rare types).
    Values,
}

/// A run of rows of one result set, column by column.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    rows: u32,
    columns: Vec<ColumnBuf>,
    bytes: u64,
}

impl Chunk {
    /// The number of rows.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The bytes the chunk holds (buffers and their heap payloads), for the memory budget.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The cell at (`row`, `col`), borrowed from the chunk (never allocates).
    ///
    /// # Panics
    /// When `row` or `col` is outside the chunk.
    #[inline]
    pub fn cell(&self, row: u32, col: u32) -> ValueRef<'_> {
        self.column(row, col).cell(row as usize)
    }

    /// The flags of the cell at (`row`, `col`).
    ///
    /// # Panics
    /// When `row` or `col` is outside the chunk.
    pub fn flags(&self, row: u32, col: u32) -> CellFlags {
        self.column(row, col).flags(row as usize)
    }

    /// A chunk of `rows`, one [`BufKind::Values`] buffer per column of `schema`: fixtures, the
    /// FakeDriver's typed scripting, tests (working since Task 1b).
    ///
    /// # Panics
    /// When a row has more or fewer values than `schema` has columns, or more than `u32::MAX`
    /// rows are given.
    pub fn from_values(schema: &ResultSchema, rows: Vec<Vec<Value>>) -> Chunk {
        let width = schema.columns.len();
        let count = u32::try_from(rows.len()).expect("a chunk holds at most u32::MAX rows");
        let mut columns: Vec<Vec<Value>> =
            (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
        for (r, row) in rows.into_iter().enumerate() {
            assert!(
                row.len() == width,
                "row {r} has {} values; the schema has {width} columns",
                row.len()
            );
            for (c, value) in row.into_iter().enumerate() {
                columns[c].push(value);
            }
        }
        let columns: Vec<ColumnBuf> = columns.into_iter().map(ColumnBuf::from_values).collect();
        let bytes = columns.iter().map(ColumnBuf::bytes).sum();
        Chunk {
            rows: count,
            columns,
            bytes,
        }
    }

    #[inline]
    fn column(&self, row: u32, col: u32) -> &ColumnBuf {
        assert!(
            row < self.rows,
            "row {row} outside a chunk of {} rows",
            self.rows
        );
        self.columns
            .get(col as usize)
            .unwrap_or_else(|| panic!("column {col} outside a chunk of {}", self.columns.len()))
    }
}
