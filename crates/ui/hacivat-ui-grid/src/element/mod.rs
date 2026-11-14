//! The grid element (HV-GRID-01, -02 fast-scroll, -03; Task 20): one custom element with
//! hitboxes, focus and the a11y declaration; regions painted body → header → gutter → corner →
//! scrollbars, each in its content mask.

mod grid;
mod layers;
mod pointer;
mod scrollbar;
mod text_cache;
mod truncate;
