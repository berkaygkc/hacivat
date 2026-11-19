// ## m4-t01c-core
//! Test doubles and fixtures for this crate's tests and its consumers' (feature `session`):
//! [`fake::FakeIntrospector`] (body Task 15) and [`big_catalog`] (the 15,000-table PostgreSQL
//! fixture, Task 16). Product code never calls them.

pub mod big_catalog;
pub mod fake;

pub use fake::FakeIntrospector;
