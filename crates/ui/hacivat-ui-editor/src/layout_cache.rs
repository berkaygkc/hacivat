//! The content-keyed shape cache (UI-EDIT-09; Task 19): a 4,096-line or 32 MB LRU keyed
//! `(text_hash, runs_hash, font_size_bits, column_window)`; long lines shape a 512-column window.
