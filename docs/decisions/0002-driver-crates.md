# ADR 0002 — Database driver crates for M2a

- Status: Accepted
- Date: 2026-11-12
- Scope: the M2a engines (PostgreSQL, MySQL and MariaDB, SQLite) and the libraries behind them

## Context

M2a connects to PostgreSQL 14+, MySQL 8.0+, MariaDB 10.6+ and SQLite through native Rust
drivers compiled into the app (HV-DRV-01). Each engine lives in its own core crate
(`hacivat-driver-pg`, `hacivat-driver-mysql`, `hacivat-driver-sqlite`) behind the `Driver` and
`RawConn` traits of `hacivat-driver`. The driver spike (`docs/superpowers/plans/2026-11-12-m2a-connect.md`,
Tech Stack) chose:

- `tokio-postgres` with `connect_raw` over an injected stream, TLS through `tokio-rustls`;
- `mysql_async` with rustls on the ring provider, reached through the loopback forwarder (R18)
  because it cannot take an injected stream;
- `rusqlite` with the bundled SQLite and `column_metadata`.

Workspace-wide rules from Task 1: rustls uses the ring provider only (`aws-lc-rs` and
`aws-lc-sys` are banned in `deny.toml`), and `webpki-roots` (CDLA-Permissive-2.0, pulled by
`mysql_async`'s rustls TLS) has a scoped licence exception.

## Rule

Each driver task records its crate here in one section headed `## m2a-tNN — <crate>`
(append-only, P41): the library and version, its licence tree, how TLS and trust work, where
secrets are copied into library-owned buffers (P49), and known defects.
