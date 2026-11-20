// ## m4-t17-mysql
//! What a pass knows of its server (HV-INTRO-07 step 1): the `Identity` section's answer,
//! read inside every snapshot, and what follows from it — the flavor (from `VERSION()`, the
//! server's own word, not the profile's), the pack version number, `lower_case_table_names`,
//! the `sql_mode` flags that change how text is quoted, the character sets the `show-create`
//! macro needs, and whether the session reads times in UTC.

use std::collections::BTreeMap;

use hacivat_meta::name::{DialectId, IdentCtx};
use hacivat_meta::{Level, Name, ServerFacts};
use hacivat_proto::DbmsKind;
use sql_ident::quote::QuoteCtx;

use crate::pack::{Mode, PackCtx, PackFlags, Params};

/// The server context of one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCtx {
    /// `DbmsKind::MySql` or `DbmsKind::MariaDb`, from `VERSION()`.
    pub flavor: DbmsKind,
    /// `VERSION()`, verbatim.
    pub version_text: Name,
    /// The pack's version number (`80046`, `110405`).
    pub version_num: u32,
    pub lower_case_table_names: u8,
    /// `@@session.sql_mode`, verbatim.
    pub sql_mode: Name,
    /// `@@character_set_system` (the character set of identifiers in the data dictionary).
    pub character_set_system: Option<Name>,
    /// `character_set_client`, `_connection` and `_results` all read `utf8mb4` (the T4 carry:
    /// the `show-create` quoting assumes a UTF-8 connection).
    pub utf8mb4: bool,
    /// `@@session.time_zone` reads `+00:00`: `CAST(<timestamp> AS CHAR)` is UTC, comparable
    /// with `UTC_TIMESTAMP(6)`.
    pub utc_session: bool,
    /// MySQL `@@server_uuid`; `None` on MariaDB.
    pub server_uuid: Option<Name>,
    /// `CURRENT_USER()` and `CURRENT_ROLE()`: part of the privilege hash.
    pub login: Option<Name>,
    pub role: Option<Name>,
}

/// `8.4.3` → `(80403, MySql)`; `11.4.5-MariaDB-ubu2404` → `(110405, MariaDb)`; MariaDB's old
/// `5.5.5-` replication prefix is skipped. A part that is not a number reads 0; each part is
/// capped at 99 (minor, patch) so the number stays ordered.
pub fn parse_version(text: &str) -> (u32, DbmsKind) {
    let flavor = if text.contains("MariaDB") {
        DbmsKind::MariaDb
    } else {
        DbmsKind::MySql
    };
    let core = text.strip_prefix("5.5.5-").unwrap_or(text);
    let mut parts = core
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0).min(4_000);
    let minor = parts.next().unwrap_or(0).min(99);
    let patch = parts.next().unwrap_or(0).min(99);
    (major * 10_000 + minor * 100 + patch, flavor)
}

/// Whether `sql_mode` (a comma-separated list) holds `flag`, ASCII-caselessly.
pub fn sql_mode_has(sql_mode: &str, flag: &str) -> bool {
    sql_mode
        .split(',')
        .any(|m| m.trim().eq_ignore_ascii_case(flag))
}

impl ServerCtx {
    /// A context for `flavor` and `version_text` with the session defaults Task 5 sets
    /// (UTF-8, UTC) and nothing else known: the pure helpers' and the tests' starting point.
    pub fn assumed(flavor: DbmsKind, version_text: &str) -> Self {
        let (version_num, parsed) = parse_version(version_text);
        let flavor = if version_text.is_empty() {
            flavor
        } else {
            parsed
        };
        ServerCtx {
            flavor,
            version_text: Name::from(version_text),
            version_num,
            lower_case_table_names: 0,
            sql_mode: Name::from(""),
            character_set_system: None,
            utf8mb4: true,
            utc_session: true,
            server_uuid: None,
            login: None,
            role: None,
        }
    }

    pub fn is_mariadb(&self) -> bool {
        self.flavor == DbmsKind::MariaDb
    }

    pub fn dialect(&self) -> DialectId {
        if self.is_mariadb() {
            DialectId::MariaDb
        } else {
            DialectId::MySql
        }
    }

    /// MySQL at or above `v` (MariaDB never counts).
    pub fn mysql_at_least(&self, v: u32) -> bool {
        !self.is_mariadb() && self.version_num >= v
    }

    /// The identifier rule of this server (HV-INTRO-14).
    pub fn ident(&self) -> IdentCtx {
        IdentCtx {
            dialect: self.dialect(),
            lower_case_table_names: self.lower_case_table_names,
            encoding: None,
        }
    }

    /// `NO_BACKSLASH_ESCAPES` is in the session's `sql_mode`.
    pub fn no_backslash_escapes(&self) -> bool {
        sql_mode_has(&self.sql_mode, "NO_BACKSLASH_ESCAPES")
    }

    /// `ANSI_QUOTES` is in the session's `sql_mode` (recorded; never trusted for quoting).
    pub fn ansi_quotes(&self) -> bool {
        sql_mode_has(&self.sql_mode, "ANSI_QUOTES")
    }

    /// The quoting context of literals this pass writes (pre-quoted column defaults).
    pub fn quote_ctx(&self) -> QuoteCtx {
        QuoteCtx {
            ansi_quotes: Some(self.ansi_quotes()),
            no_backslash_escapes: Some(self.no_backslash_escapes()),
            standard_conforming_strings: None,
        }
    }

    /// MySQL 8.0+ gets the R-M4-39 marker path; MariaDB is `Full` per namespace (R-M4-10).
    pub fn incremental_supported(&self) -> bool {
        self.mysql_at_least(80_000)
    }

    /// `VIEW_TABLE_USAGE` exists (MySQL 8.0.13+).
    pub fn has_view_usage(&self) -> bool {
        self.mysql_at_least(80_013)
    }

    /// `CHECK_CONSTRAINTS` exists (MySQL 8.0.16+, MariaDB 10.2+).
    pub fn has_checks(&self) -> bool {
        self.is_mariadb() || self.mysql_at_least(80_016)
    }

    /// The pack context of one section rendering.
    pub fn pack_ctx(&self, mode: Mode, level: Level, params: Params) -> PackCtx {
        PackCtx {
            dialect: self.dialect(),
            version: self.version_num,
            flavor: self.flavor,
            mode,
            level,
            flags: if self.is_mariadb() {
                PackFlags::MARIADB
            } else {
                PackFlags::empty()
            },
            params,
        }
    }

    /// The model's server facts (`default_collation` from `ListNamespaces`).
    pub fn facts(&self, default_collation: BTreeMap<Name, Name>) -> ServerFacts {
        let mut f = ServerFacts::new(self.flavor, self.ident());
        f.version = self.version_text.clone();
        f.version_num = self.version_num;
        f.default_collation = default_collation;
        f.no_backslash_escapes = Some(self.no_backslash_escapes());
        f.ansi_quotes = Some(self.ansi_quotes());
        f
    }
}

/// What `ServerIdentity` read (`@@server_id`, `@@hostname`, `@@port`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    pub server_id: Option<Name>,
    pub host_name: Option<Name>,
    pub port: Option<Name>,
}

/// The server's identity for the cache (ruling R-W3-3; `ServerFingerprint`, XXH3-128):
///
/// - **MySQL**: `@@server_uuid` (generated once per data directory and kept in `auto.cnf`;
///   MySQL 8.4 manual, "Replication and Binary Logging Options and Variables") and the version.
///   A restore into another data directory, or a swapped server, changes it.
/// - **MariaDB** has no server UUID: `@@server_id` (an administrator's number, 1 by default,
///   so not unique alone), `@@hostname` and `@@port` together with the version (MariaDB
///   Knowledge Base, "Server System Variables").
///
/// `None` (the seam's `Unknown`) when a fact it needs could not be read: never a guess.
pub fn fingerprint(srv: &ServerCtx, host: &HostFacts) -> Option<u128> {
    let mut facts: Vec<&str> = vec![srv.flavor.id(), &srv.version_text];
    if srv.version_text.is_empty() {
        return None;
    }
    if srv.is_mariadb() {
        facts.push(host.server_id.as_deref()?);
        facts.push(host.host_name.as_deref()?);
        facts.push(host.port.as_deref()?);
    } else {
        facts.push(srv.server_uuid.as_deref().filter(|u| !u.is_empty())?);
    }
    Some(twox_hash::XxHash3_128::oneshot(facts.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_uses_the_server_uuid_or_mariadbs_host_facts_and_never_guesses() {
        let mut my = ServerCtx::assumed(DbmsKind::MySql, "8.4.3");
        assert_eq!(fingerprint(&my, &HostFacts::default()), None);
        my.server_uuid = Some(Name::from("u-1"));
        let a = fingerprint(&my, &HostFacts::default()).expect("uuid and version");
        my.server_uuid = Some(Name::from("u-2"));
        assert_ne!(Some(a), fingerprint(&my, &HostFacts::default()));
        let maria = ServerCtx::assumed(DbmsKind::MariaDb, "11.4.5-MariaDB");
        let host = HostFacts {
            server_id: Some(Name::from("1")),
            host_name: Some(Name::from("db")),
            port: Some(Name::from("3306")),
        };
        let b = fingerprint(&maria, &host).expect("host facts");
        let other_port = HostFacts {
            port: Some(Name::from("3307")),
            ..host.clone()
        };
        assert_ne!(Some(b), fingerprint(&maria, &other_port));
        assert_eq!(
            fingerprint(
                &maria,
                &HostFacts {
                    host_name: None,
                    ..host
                }
            ),
            None
        );
    }

    #[test]
    fn versions_and_flavors_parse() {
        assert_eq!(parse_version("8.4.3"), (80_403, DbmsKind::MySql));
        assert_eq!(parse_version("8.0.46"), (80_046, DbmsKind::MySql));
        assert_eq!(
            parse_version("11.4.5-MariaDB-ubu2404"),
            (110_405, DbmsKind::MariaDb)
        );
        assert_eq!(
            parse_version("5.5.5-10.6.21-MariaDB-log"),
            (100_621, DbmsKind::MariaDb)
        );
        assert_eq!(parse_version(""), (0, DbmsKind::MySql));
    }

    #[test]
    fn sql_mode_flags_are_exact_words() {
        let m = "STRICT_TRANS_TABLES,no_backslash_escapes,ANSI";
        assert!(sql_mode_has(m, "NO_BACKSLASH_ESCAPES"));
        assert!(!sql_mode_has(m, "ANSI_QUOTES"));
        assert!(sql_mode_has(m, "ANSI"));
    }

    #[test]
    fn only_mysql_8_is_incremental_and_mariadb_versions_never_pass_mysql_gates() {
        let my = ServerCtx::assumed(DbmsKind::MySql, "8.0.46");
        assert!(my.incremental_supported() && my.has_view_usage() && my.has_checks());
        let maria = ServerCtx::assumed(DbmsKind::MySql, "11.4.5-MariaDB");
        assert_eq!(maria.flavor, DbmsKind::MariaDb);
        assert!(!maria.incremental_supported() && !maria.has_view_usage());
        assert!(maria.has_checks());
        let old = ServerCtx::assumed(DbmsKind::MySql, "8.0.12");
        assert!(old.incremental_supported() && !old.has_view_usage() && !old.has_checks());
    }
}
