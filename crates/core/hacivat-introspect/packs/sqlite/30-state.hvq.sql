-- Hacivat query pack: SQLite, file 30-state (HV-INTRO-04, R-M4-32).
-- Contents: schema_version and digest stamps for check_state. Sections are written by Task 18 from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
--
-- ## m4-t18-sqlite (HV-INTRO-08, -15). The state of a SQLite schema is schema-level: its
-- `schema_version` (the header's schema cookie) plus the XXH3-128 digest of its sorted
-- `sqlite_schema` rows, which the introspector computes from the `Objects…` sections of
-- `10-core`. The table-valued `pragma_schema_version` takes no schema argument, so it reads
-- `main` only; `temp` and attached schemas are stamped by their digest alone.
-- `ApplicationIdMain` is the file's identity for the cache's server fingerprint (R-W3-3):
-- main's application_id (a file format's own tag; 0 for a plain database) with the file's
-- canonical path, which the introspector hashes.

--- name: SchemaVersionMain ---
--- returns: schema_version i64 ---
--- joinable ---
SELECT v.schema_version AS schema_version
FROM pragma_schema_version v

--- name: ApplicationIdMain ---
--- returns: application_id i64 ---
--- joinable ---
SELECT a.application_id AS application_id
FROM pragma_application_id a
