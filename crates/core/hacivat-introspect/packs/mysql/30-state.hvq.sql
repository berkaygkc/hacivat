-- Hacivat query pack: MySQL and MariaDB, file 30-state (HV-INTRO-04, R-M4-32).
-- Contents: StateOf<Kind> stamps for check_state. Sections are written by Task 17 from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
-- ## m4-t17-mysql (M4 Task 17): the raw state marks of named objects (`Introspector::stamps`),
-- `joinable` so M7's drift check can read them inside its own transaction (R-M4-29). The
-- table stamp is R-M4-39's `CREATE_TIME | max(TRIGGERS.CREATED) | trigger count`: one row per
-- trigger (or one row with NULL trigger columns), folded by the client, so the sections hold
-- no aggregate under the list bind. A view's stamp is the hash of its definition columns (the
-- same expression as `ViewMarks` in 10-core). Times are text in the session's UTC.

--- name: StateOfRelation ---
--- returns: table_name text, table_type text, create_time text, view_hash text, trigger_name text, trigger_created text ---
--- joinable ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type,
  CAST(t.CREATE_TIME AS CHAR) AS create_time,
  SHA2(CONCAT_WS('|', v.VIEW_DEFINITION, v.CHECK_OPTION, v.IS_UPDATABLE, v.SECURITY_TYPE, v.DEFINER), 256) AS view_hash,
  g.TRIGGER_NAME AS trigger_name, CAST(g.CREATED AS CHAR) AS trigger_created
FROM INFORMATION_SCHEMA.TABLES t
LEFT JOIN INFORMATION_SCHEMA.VIEWS v
  ON v.TABLE_SCHEMA = t.TABLE_SCHEMA AND v.TABLE_NAME = t.TABLE_NAME
LEFT JOIN INFORMATION_SCHEMA.TRIGGERS g
  ON g.EVENT_OBJECT_SCHEMA = t.TABLE_SCHEMA AND g.EVENT_OBJECT_TABLE = t.TABLE_NAME
WHERE t.TABLE_SCHEMA = :schema AND t.TABLE_NAME IN :[names]

--- name: StateOfRoutine ---
--- returns: specific_name text, routine_type text, last_altered text ---
--- joinable ---
SELECT r.SPECIFIC_NAME AS specific_name, r.ROUTINE_TYPE AS routine_type,
  CAST(r.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.ROUTINES r
WHERE r.ROUTINE_SCHEMA = :schema AND r.SPECIFIC_NAME IN :[names]

--- name: StateOfTrigger ---
--- returns: trigger_name text, table_name text, created text ---
--- joinable ---
SELECT g.TRIGGER_NAME AS trigger_name, g.EVENT_OBJECT_TABLE AS table_name,
  CAST(g.CREATED AS CHAR) AS created
FROM INFORMATION_SCHEMA.TRIGGERS g
WHERE g.TRIGGER_SCHEMA = :schema AND g.TRIGGER_NAME IN :[names]

--- name: StateOfEvent ---
--- returns: event_name text, last_altered text ---
--- joinable ---
SELECT e.EVENT_NAME AS event_name, CAST(e.LAST_ALTERED AS CHAR) AS last_altered
FROM INFORMATION_SCHEMA.EVENTS e
WHERE e.EVENT_SCHEMA = :schema AND e.EVENT_NAME IN :[names]
