//! The display cache of the grid adapter (Task 26): keyed `(generation, row, col,
//! settings_version)`, 64k entries, LRU by frame generation.
//!
//! The key has **no store version**. A streaming result publishes a new version on every
//! append; rows are immutable inside a generation, so a formatted cell stays right while rows
//! arrive, and keying on the version would empty the cache on every publish (R-M3-12). The
//! generation is the adapter's own (it moves on every re-run into the same tab); the settings
//! version moves whenever the display settings do.
//!
//! Recency is the frame: [`DisplayCache::begin_frame`] starts a frame, a hit marks its entry as
//! used in it, and a full cache drops the oldest frames' entries down to 7/8 of its capacity,
//! never an entry of the current frame (the grid's `FrameLru`).

use hacivat_ui_grid::CellContent;
use hacivat_ui_grid::element::text_cache::FrameLru;

/// The entries the display cache holds (R-M3-12).
pub const DISPLAY_CACHE_CAPACITY: usize = 65_536;

/// What a formatted cell is cached under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayKey {
    /// The adapter's generation (a re-run), never the store's version.
    pub generation: u64,
    pub row: u64,
    pub col: u32,
    /// `DisplaySettings::version`.
    pub settings_version: u32,
}

/// Formatted cells by [`DisplayKey`].
#[derive(Debug)]
pub struct DisplayCache {
    lru: FrameLru<DisplayKey, CellContent>,
    /// Cells formatted (misses) since the cache was built.
    formats: u64,
}

impl Default for DisplayCache {
    fn default() -> Self {
        DisplayCache::with_capacity(DISPLAY_CACHE_CAPACITY)
    }
}

impl DisplayCache {
    /// A cache of [`DISPLAY_CACHE_CAPACITY`] entries.
    pub fn new() -> Self {
        DisplayCache::default()
    }

    /// A cache of `capacity` entries (tests).
    pub fn with_capacity(capacity: usize) -> Self {
        DisplayCache {
            lru: FrameLru::new(capacity),
            formats: 0,
        }
    }

    /// Starts a frame (the unit of recency).
    pub fn begin_frame(&mut self) {
        self.lru.begin_frame();
    }

    /// The cell cached under `key`, formatted by `format` on a miss. A hit clones the cached
    /// cell: reference-count bumps, no allocation.
    pub fn get_or_format(
        &mut self,
        key: DisplayKey,
        format: impl FnOnce() -> CellContent,
    ) -> CellContent {
        let (cell, miss) = self.lru.get_or_insert_with(key, format);
        let cell = cell.clone();
        self.formats += u64::from(miss);
        cell
    }

    /// Cells formatted (cache misses) so far.
    pub fn formats(&self) -> u64 {
        self.formats
    }

    pub fn len(&self) -> usize {
        self.lru.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lru.is_empty()
    }

    /// Forgets every cell (the UI language changed: chips and decode-error texts are Fluent).
    pub fn clear(&mut self) {
        self.lru.clear();
    }
}
