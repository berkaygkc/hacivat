-- Hacivat query pack: MySQL and MariaDB, file 10-core (HV-INTRO-04, R-M4-32).
-- Contents: identity, schemata, existence with R-M4-39 markers and the core object passes. Sections are written by Task 17 from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
-- ## m4-t17-mysql (M4 Task 17): written from the MySQL 8.0/8.4 Reference Manual chapter
-- "INFORMATION_SCHEMA Tables" and the MariaDB Knowledge Base "Information Schema Tables".
-- Rules every section follows:
-- - Names reach the server only as binds; `INFORMATION_SCHEMA.<NAME>` is upper case; no
--   `UPDATE_TIME`, no `GROUP_CONCAT` (one row per column or key part, grouped by the client),
--   no case folding, and `information_schema_stats_expiry` is never touched.
-- - Every column has an alias equal to its declared name, so the declared `returns` are the
--   server's column names (the CI matrix compares them).
-- - The version is MySQL's `80046` style (major * 10000 + minor * 100 + patch); MariaDB
--   versions are larger than every MySQL 8 version, so every MySQL version gate is also
--   gated on `not flag(mariadb)`.
-- - `mode == incremental` is the re-read of named objects (an incremental pass's changed
--   objects, a fragment): one list bind `:[names]` on the object's own name column.
-- - Time markers are cast to text (`CAST(x AS CHAR)`) and compared by the client in UTC: the
--   metadata session runs with `time_zone = '+00:00'` (Task 5), and `Identity` reads it back.
-- - R-M4-37: `SHOW CREATE` takes no bind parameter, so the `ShowCreate*` sections name the
--   object through the identifier macro, rendered only through `sql_ident::quote` (backticks
--   doubled, NUL refused). The server names those columns (`Table`, `Create Table`, …); the
--   declared names give their positions.

--- name: Identity ---
--- returns: version text, lower_case_table_names u32, sql_mode text, character_set_system text, character_set_client text, character_set_connection text, character_set_results text, time_zone text, login_user text, login_role text ---
SELECT VERSION() AS version,
  @@lower_case_table_names AS lower_case_table_names,
  @@session.sql_mode AS sql_mode,
  @@character_set_system AS character_set_system,
  @@session.character_set_client AS character_set_client,
  @@session.character_set_connection AS character_set_connection,
  @@session.character_set_results AS character_set_results,
  @@session.time_zone AS time_zone,
  CURRENT_USER() AS login_user,
  CURRENT_ROLE() AS login_role

--- name: ServerUuid ---
--- returns: server_uuid text ---
--# if not flag(mariadb)
SELECT @@server_uuid AS server_uuid
--# end

--- name: ServerIdentity ---
--- returns: server_id text, host_name text, port_number text ---
SELECT @@server_id AS server_id, @@hostname AS host_name, @@port AS port_number

--- name: Watermark ---
--- returns: at_utc text ---
SELECT CAST(UTC_TIMESTAMP(6) AS CHAR) AS at_utc

--- name: ListNamespaces ---
--- returns: schema_name text, default_charset text, default_collation text ---
SELECT s.SCHEMA_NAME AS schema_name,
  s.DEFAULT_CHARACTER_SET_NAME AS default_charset,
  s.DEFAULT_COLLATION_NAME AS default_collation
FROM INFORMATION_SCHEMA.SCHEMATA s

--- name: Privileges ---
--- returns: scope text, grantee text, table_name text, column_name text, privilege_type text, is_grantable text ---
SELECT 'user' AS scope, u.GRANTEE AS grantee, NULL AS table_name, NULL AS column_name,
  u.PRIVILEGE_TYPE AS privilege_type, u.IS_GRANTABLE AS is_grantable
FROM INFORMATION_SCHEMA.USER_PRIVILEGES u
UNION ALL
SELECT 'schema', s.GRANTEE, NULL, NULL, s.PRIVILEGE_TYPE, s.IS_GRANTABLE
FROM INFORMATION_SCHEMA.SCHEMA_PRIVILEGES s
WHERE s.TABLE_SCHEMA = :schema
UNION ALL
SELECT 'table', p.GRANTEE, p.TABLE_NAME, NULL, p.PRIVILEGE_TYPE, p.IS_GRANTABLE
FROM INFORMATION_SCHEMA.TABLE_PRIVILEGES p
WHERE p.TABLE_SCHEMA = :schema
UNION ALL
SELECT 'column', c.GRANTEE, c.TABLE_NAME, c.COLUMN_NAME, c.PRIVILEGE_TYPE, c.IS_GRANTABLE
FROM INFORMATION_SCHEMA.COLUMN_PRIVILEGES c
WHERE c.TABLE_SCHEMA = :schema

--- name: TableMarks ---
--- returns: table_name text, table_type text, create_time text ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type,
  CAST(t.CREATE_TIME AS CHAR) AS create_time
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_SCHEMA = :schema

--- name: ViewMarks ---
--- returns: table_name text, view_hash text ---
SELECT v.TABLE_NAME AS table_name,
  SHA2(CONCAT_WS('|', v.VIEW_DEFINITION, v.CHECK_OPTION, v.IS_UPDATABLE, v.SECURITY_TYPE, v.DEFINER), 256) AS view_hash
FROM INFORMATION_SCHEMA.VIEWS v
WHERE v.TABLE_SCHEMA = :schema

--- name: TriggerMarks ---
--- returns: trigger_name text, table_name text, created text ---
SELECT g.TRIGGER_NAME AS trigger_name, g.EVENT_OBJECT_TABLE AS table_name,
  CAST(g.CREATED AS CHAR) AS created
FROM INFORMATION_SCHEMA.TRIGGERS g
WHERE g.TRIGGER_SCHEMA = :schema

--- name: RoutineMarks ---
--- returns: specific_name text, routine_type text, last_altered text ---
SELECT r.SPECIFIC_NAME AS specific_name, r.ROUTINE_TYPE AS routine_type,
  CAST(r.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.ROUTINES r
WHERE r.ROUTINE_SCHEMA = :schema

--- name: EventMarks ---
--- returns: event_name text, last_altered text ---
SELECT e.EVENT_NAME AS event_name, CAST(e.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.EVENTS e
WHERE e.EVENT_SCHEMA = :schema

--- name: ViewUsage ---
--- returns: view_name text, table_schema text, table_name text ---
--# if not flag(mariadb) and version >= 80013
SELECT u.VIEW_NAME AS view_name, u.TABLE_SCHEMA AS table_schema, u.TABLE_NAME AS table_name
FROM INFORMATION_SCHEMA.VIEW_TABLE_USAGE u
WHERE u.VIEW_SCHEMA = :schema
--# end

--- name: Tables ---
--- returns: table_name text, table_type text, engine_name text, table_collation text, create_options text, table_comment text, create_time text, table_rows u64 ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type, t.ENGINE AS engine_name,
  t.TABLE_COLLATION AS table_collation, t.CREATE_OPTIONS AS create_options,
  t.TABLE_COMMENT AS table_comment, CAST(t.CREATE_TIME AS CHAR) AS create_time,
  t.TABLE_ROWS AS table_rows
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND t.TABLE_NAME IN :[names]
--# end

--- name: TableNamesLike ---
--- returns: table_name text, table_type text ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_SCHEMA = :schema AND t.TABLE_NAME LIKE :name_like ESCAPE :like_escape

--- name: Views ---
--- returns: table_name text, check_option text, is_updatable text, security_type text, definer text, view_algorithm text, view_hash text ---
SELECT v.TABLE_NAME AS table_name, v.CHECK_OPTION AS check_option,
  v.IS_UPDATABLE AS is_updatable, v.SECURITY_TYPE AS security_type, v.DEFINER AS definer,
--# if flag(mariadb)
  v.ALGORITHM AS view_algorithm,
--# else
  NULL AS view_algorithm,
--# end
  SHA2(CONCAT_WS('|', v.VIEW_DEFINITION, v.CHECK_OPTION, v.IS_UPDATABLE, v.SECURITY_TYPE, v.DEFINER), 256) AS view_hash
FROM INFORMATION_SCHEMA.VIEWS v
WHERE v.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND v.TABLE_NAME IN :[names]
--# end

--- name: Columns ---
--- returns: table_name text, column_name text, ordinal_position u32, column_default text, is_nullable text, data_type text, column_type text, character_set_name text, collation_name text, extra text, column_comment text, generation_expression text ---
SELECT c.TABLE_NAME AS table_name, c.COLUMN_NAME AS column_name,
  c.ORDINAL_POSITION AS ordinal_position, c.COLUMN_DEFAULT AS column_default,
  c.IS_NULLABLE AS is_nullable, c.DATA_TYPE AS data_type, c.COLUMN_TYPE AS column_type,
  c.CHARACTER_SET_NAME AS character_set_name, c.COLLATION_NAME AS collation_name,
  c.EXTRA AS extra, c.COLUMN_COMMENT AS column_comment,
  c.GENERATION_EXPRESSION AS generation_expression
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND c.TABLE_NAME IN :[names]
--# end

--- name: Statistics ---
--- returns: table_name text, index_name text, non_unique u32, seq_in_index u32, column_name text, collation_order text, sub_part u32, index_type text, index_comment text, is_visible text, expression text ---
SELECT s.TABLE_NAME AS table_name, s.INDEX_NAME AS index_name,
  s.NON_UNIQUE AS non_unique, s.SEQ_IN_INDEX AS seq_in_index,
  s.COLUMN_NAME AS column_name, s.COLLATION AS collation_order, s.SUB_PART AS sub_part,
  s.INDEX_TYPE AS index_type, s.INDEX_COMMENT AS index_comment,
--# if not flag(mariadb)
  s.IS_VISIBLE AS is_visible,
--# elif version >= 100600
  CASE s.IGNORED WHEN 'YES' THEN 'NO' ELSE 'YES' END AS is_visible,
--# else
  'YES' AS is_visible,
--# end
--# if not flag(mariadb) and version >= 80013
  s.EXPRESSION AS expression
--# else
  NULL AS expression
--# end
FROM INFORMATION_SCHEMA.STATISTICS s
WHERE s.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND s.TABLE_NAME IN :[names]
--# end

--- name: TableConstraints ---
--- returns: table_name text, constraint_name text, constraint_type text, enforced text ---
SELECT tc.TABLE_NAME AS table_name, tc.CONSTRAINT_NAME AS constraint_name,
  tc.CONSTRAINT_TYPE AS constraint_type,
--# if not flag(mariadb) and version >= 80016
  tc.ENFORCED AS enforced
--# else
  'YES' AS enforced
--# end
FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
WHERE tc.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND tc.TABLE_NAME IN :[names]
--# end

--- name: KeyColumnUsage ---
--- returns: table_name text, constraint_name text, column_name text, ordinal_position u32, referenced_table_schema text, referenced_table_name text, referenced_column_name text ---
SELECT k.TABLE_NAME AS table_name, k.CONSTRAINT_NAME AS constraint_name,
  k.COLUMN_NAME AS column_name, k.ORDINAL_POSITION AS ordinal_position,
  k.REFERENCED_TABLE_SCHEMA AS referenced_table_schema,
  k.REFERENCED_TABLE_NAME AS referenced_table_name,
  k.REFERENCED_COLUMN_NAME AS referenced_column_name
FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
WHERE k.TABLE_SCHEMA = :schema
--# if mode == incremental
  AND k.TABLE_NAME IN :[names]
--# end

--- name: ReferentialConstraints ---
--- returns: table_name text, constraint_name text, unique_constraint_schema text, referenced_table_name text, unique_constraint_name text, match_option text, update_rule text, delete_rule text ---
SELECT r.TABLE_NAME AS table_name, r.CONSTRAINT_NAME AS constraint_name,
  r.UNIQUE_CONSTRAINT_SCHEMA AS unique_constraint_schema,
  r.REFERENCED_TABLE_NAME AS referenced_table_name,
  r.UNIQUE_CONSTRAINT_NAME AS unique_constraint_name, r.MATCH_OPTION AS match_option,
  r.UPDATE_RULE AS update_rule, r.DELETE_RULE AS delete_rule
FROM INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS r
WHERE r.CONSTRAINT_SCHEMA = :schema
--# if mode == incremental
  AND r.TABLE_NAME IN :[names]
--# end

--- name: CheckConstraints ---
--- returns: table_name text, constraint_name text, check_clause text ---
--# if flag(mariadb) or version >= 80016
SELECT tc.TABLE_NAME AS table_name, cc.CONSTRAINT_NAME AS constraint_name,
  cc.CHECK_CLAUSE AS check_clause
FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc
JOIN INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
  ON tc.CONSTRAINT_SCHEMA = cc.CONSTRAINT_SCHEMA AND tc.CONSTRAINT_NAME = cc.CONSTRAINT_NAME
  AND tc.CONSTRAINT_TYPE = 'CHECK'
--# if flag(mariadb)
  AND tc.TABLE_NAME = cc.TABLE_NAME
--# end
WHERE cc.CONSTRAINT_SCHEMA = :schema
--# if mode == incremental
  AND tc.TABLE_NAME IN :[names]
--# end
--# end

--- name: Routines ---
--- returns: specific_name text, routine_name text, routine_type text, dtd_identifier text, is_deterministic text, sql_data_access text, security_type text, definer text, routine_comment text, last_altered text ---
SELECT r.SPECIFIC_NAME AS specific_name, r.ROUTINE_NAME AS routine_name,
  r.ROUTINE_TYPE AS routine_type, r.DTD_IDENTIFIER AS dtd_identifier,
  r.IS_DETERMINISTIC AS is_deterministic, r.SQL_DATA_ACCESS AS sql_data_access,
  r.SECURITY_TYPE AS security_type, r.DEFINER AS definer,
  r.ROUTINE_COMMENT AS routine_comment, CAST(r.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.ROUTINES r
WHERE r.ROUTINE_SCHEMA = :schema
--# if mode == incremental
  AND r.SPECIFIC_NAME IN :[names]
--# end

--- name: Parameters ---
--- returns: specific_name text, routine_type text, ordinal_position u32, parameter_mode text, parameter_name text, dtd_identifier text ---
SELECT p.SPECIFIC_NAME AS specific_name, p.ROUTINE_TYPE AS routine_type,
  p.ORDINAL_POSITION AS ordinal_position, p.PARAMETER_MODE AS parameter_mode,
  p.PARAMETER_NAME AS parameter_name, p.DTD_IDENTIFIER AS dtd_identifier
FROM INFORMATION_SCHEMA.PARAMETERS p
WHERE p.SPECIFIC_SCHEMA = :schema AND p.ORDINAL_POSITION > 0
--# if mode == incremental
  AND p.SPECIFIC_NAME IN :[names]
--# end

--- name: Triggers ---
--- returns: trigger_name text, table_name text, action_timing text, event_manipulation text, action_order u32, created text, definer text ---
SELECT g.TRIGGER_NAME AS trigger_name, g.EVENT_OBJECT_TABLE AS table_name,
  g.ACTION_TIMING AS action_timing, g.EVENT_MANIPULATION AS event_manipulation,
  g.ACTION_ORDER AS action_order, CAST(g.CREATED AS CHAR) AS created, g.DEFINER AS definer
FROM INFORMATION_SCHEMA.TRIGGERS g
WHERE g.TRIGGER_SCHEMA = :schema
--# if mode == incremental
  AND g.EVENT_OBJECT_TABLE IN :[names]
--# end

--- name: Events ---
--- returns: event_name text, event_type text, execute_at text, interval_value text, interval_field text, status text, on_completion text, event_comment text, definer text, last_altered text ---
SELECT e.EVENT_NAME AS event_name, e.EVENT_TYPE AS event_type,
  CAST(e.EXECUTE_AT AS CHAR) AS execute_at, e.INTERVAL_VALUE AS interval_value,
  e.INTERVAL_FIELD AS interval_field, e.STATUS AS status, e.ON_COMPLETION AS on_completion,
  e.EVENT_COMMENT AS event_comment, e.DEFINER AS definer,
  CAST(e.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.EVENTS e
WHERE e.EVENT_SCHEMA = :schema
--# if mode == incremental
  AND e.EVENT_NAME IN :[names]
--# end

--- name: ShowCreateTable ---
--- returns: name text, create_statement text ---
--- show-create ---
SHOW CREATE TABLE #{ident:schema}.#{ident:name}

--- name: ShowCreateView ---
--- returns: name text, create_statement text, character_set_client text, collation_connection text ---
--- show-create ---
SHOW CREATE VIEW #{ident:schema}.#{ident:name}

--- name: ShowCreateProcedure ---
--- returns: name text, sql_mode text, create_statement text, character_set_client text, collation_connection text, database_collation text ---
--- show-create ---
SHOW CREATE PROCEDURE #{ident:schema}.#{ident:name}

--- name: ShowCreateFunction ---
--- returns: name text, sql_mode text, create_statement text, character_set_client text, collation_connection text, database_collation text ---
--- show-create ---
SHOW CREATE FUNCTION #{ident:schema}.#{ident:name}

--- name: ShowCreateTrigger ---
--- returns: name text, sql_mode text, create_statement text, character_set_client text, collation_connection text, database_collation text, created text ---
--- show-create ---
SHOW CREATE TRIGGER #{ident:schema}.#{ident:name}

--- name: ShowCreateEvent ---
--- returns: name text, sql_mode text, time_zone text, create_statement text, character_set_client text, collation_connection text, database_collation text ---
--- show-create ---
SHOW CREATE EVENT #{ident:schema}.#{ident:name}
