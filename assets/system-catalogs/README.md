# System-catalog snapshots

The built-in objects of every supported server version, so the explorer and completion know
system tables, views, functions, types, operators and settings offline (spec §1.3, §4
"Introspection and metadata model"; L7, D-017; ADR 0008).

| File | What |
|---|---|
| `<dbms>-<line>.hvsnap.zst` | One server line's system catalog: canonical JSON format 1 in a zstd frame (level 19, single-threaded, no dictionary, checksum on). PostgreSQL 14–18 by major; MySQL 8.0, 8.4 and MariaDB 10.6, 10.11, 11.4 by major.minor. No SQLite snapshot (R-M4-6). |
| `manifest.toml` | Per file: DBMS, line, exact server version, image digest from `docker/m2a/compose.yml`, format, JSON and zst byte counts, XXH3-64 digests, object counts, and `excluded = [...]` (every named exclusion rule). |
| `virtual-fks.toml` | Hand-authored virtual foreign keys between system tables (R-M4-12). |

Rules:

- **Generated, never edited by hand.** `cargo xtask minicat collect` reads the pinned Docker
  images through Hacivat's own introspectors (R-M4-33, R-M4-44) and builds the files;
  `cargo xtask minicat check` verifies the manifest offline; the `minicat-verify` CI job
  regenerates them from a fresh stack and byte-compares (R-M4-17). amd64 CI is authoritative
  for the bytes.
- **Clean room.** Nothing here comes from another tool's snapshots or catalog SQL (alg/02
  §0.2, HV-INTRO-16).
- **Exact selection.** A server version without a snapshot introspects its system schemas
  live (R-M4-6).
- Decoding lives in `hacivat_meta::system`; only `hacivat-app` embeds these files (feature
  `embedded-system`, `cargo xtask check-deps` rule `EmbeddedSystemOnlyFromApp`).

Empty until M4 Task 23 generates the first set.
