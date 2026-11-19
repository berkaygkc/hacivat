//! Constraint placement (HV-DDL-09): `InsideColumn | InsideTable (default) | AfterTable`;
//! cycles go after the table (SQLite: inside, with a notice); dialect limits override the mode
//! (MySQL FKs are never inline). Body: Task 7.
