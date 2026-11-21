-- Hacivat query pack: MySQL and MariaDB, file 20-facets (HV-INTRO-04, R-M4-32).
-- Contents: grants, dependencies, statistics and server-level sections (R-M4-30). Sections are written by Task 17b from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
-- ## m4-t17b-mysql-facets (M4 Task 17b): written from the MySQL 8.0/8.4 Reference Manual
-- chapters "Grant Tables", "INFORMATION_SCHEMA VIEW_TABLE_USAGE / VIEW_ROUTINE_USAGE /
-- APPLICABLE_ROLES / USER_PRIVILEGES Table" and "Server System Variables" (partial_revokes),
-- and the MariaDB Knowledge Base "Information Schema APPLICABLE_ROLES Table".
-- Rules every section follows (as in 10-core):
-- - Names reach the server only as binds; every column has an alias equal to its declared
--   name; no case folding; the client compares every returned name byte for byte.
-- - Grants are read from the grant tables themselves (mysql.db, mysql.tables_priv,
--   mysql.columns_priv), never from the INFORMATION_SCHEMA privilege views: those show other
--   accounts only to a login with SELECT on the mysql schema, and print the grantee as one
--   unescaped quoted string. A grant table read either answers every row or is refused, so a
--   refusal is the read being incomplete. No section names mysql.user or mysql.global_priv
--   (their authentication columns are never read).
-- - Roles come from APPLICABLE_ROLES (no password column anywhere).

--- name: GrantsTable ---
--- returns: user_name text, host_name text, table_name text, table_priv text ---
SELECT p.User AS user_name, p.Host AS host_name, p.Table_name AS table_name,
  CAST(p.Table_priv AS CHAR) AS table_priv
FROM mysql.tables_priv p
WHERE p.Db = :schema
  AND p.Table_name IN :[names]

--- name: GrantsColumn ---
--- returns: user_name text, host_name text, table_name text, column_name text, column_priv text ---
SELECT c.User AS user_name, c.Host AS host_name, c.Table_name AS table_name,
  c.Column_name AS column_name, CAST(c.Column_priv AS CHAR) AS column_priv
FROM mysql.columns_priv c
WHERE c.Db = :schema
  AND c.Table_name IN :[names]

--- name: GrantsSchema ---
--- returns: user_name text, host_name text, db_name text, select_priv text, insert_priv text, update_priv text, delete_priv text, create_priv text, drop_priv text, grant_priv text, references_priv text, index_priv text, alter_priv text, create_tmp_table_priv text, lock_tables_priv text, create_view_priv text, show_view_priv text, create_routine_priv text, alter_routine_priv text, execute_priv text, event_priv text, trigger_priv text ---
--# if not flag(mariadb) and version < 90000
SELECT m.User AS user_name, m.Host AS host_name, m.Db AS db_name,
  m.Select_priv AS select_priv, m.Insert_priv AS insert_priv, m.Update_priv AS update_priv,
  m.Delete_priv AS delete_priv, m.Create_priv AS create_priv, m.Drop_priv AS drop_priv,
  m.Grant_priv AS grant_priv, m.References_priv AS references_priv,
  m.Index_priv AS index_priv, m.Alter_priv AS alter_priv,
  m.Create_tmp_table_priv AS create_tmp_table_priv,
  m.Lock_tables_priv AS lock_tables_priv, m.Create_view_priv AS create_view_priv,
  m.Show_view_priv AS show_view_priv, m.Create_routine_priv AS create_routine_priv,
  m.Alter_routine_priv AS alter_routine_priv, m.Execute_priv AS execute_priv,
  m.Event_priv AS event_priv, m.Trigger_priv AS trigger_priv
FROM mysql.db m
WHERE m.Db IN :[names]
--# end

--- name: DepsViews ---
--- returns: view_name text ---
SELECT v.TABLE_NAME AS view_name
FROM INFORMATION_SCHEMA.VIEWS v
WHERE v.TABLE_SCHEMA = :schema
  AND v.TABLE_NAME IN :[names]

--- name: DepsViewTables ---
--- returns: view_name text, table_schema text, table_name text, table_type text ---
--# if not flag(mariadb) and version >= 80013
SELECT u.VIEW_NAME AS view_name, u.TABLE_SCHEMA AS table_schema,
  u.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type
FROM INFORMATION_SCHEMA.VIEW_TABLE_USAGE u
LEFT JOIN INFORMATION_SCHEMA.TABLES t
  ON t.TABLE_SCHEMA = u.TABLE_SCHEMA AND t.TABLE_NAME = u.TABLE_NAME
WHERE u.VIEW_SCHEMA = :schema
  AND u.VIEW_NAME IN :[names]
--# end

--- name: DepsViewRoutines ---
--- returns: view_name text, specific_schema text, specific_name text ---
--# if not flag(mariadb) and version >= 80013
SELECT u.TABLE_NAME AS view_name, u.SPECIFIC_SCHEMA AS specific_schema,
  u.SPECIFIC_NAME AS specific_name
FROM INFORMATION_SCHEMA.VIEW_ROUTINE_USAGE u
WHERE u.TABLE_SCHEMA = :schema
  AND u.TABLE_NAME IN :[names]
--# end

--- name: DepsCoverage ---
--- returns: grantee text, privilege_type text ---
SELECT u.GRANTEE AS grantee, u.PRIVILEGE_TYPE AS privilege_type
FROM INFORMATION_SCHEMA.USER_PRIVILEGES u

--- name: PartialRevokes ---
--- returns: partial_revokes text ---
--# if not flag(mariadb) and version >= 80016
SELECT CAST(@@global.partial_revokes AS CHAR) AS partial_revokes
--# end

--- name: ListRoles ---
--- returns: role_name text, role_host text ---
--# if flag(mariadb)
SELECT a.ROLE_NAME AS role_name, NULL AS role_host
FROM INFORMATION_SCHEMA.APPLICABLE_ROLES a
--# elif version >= 80019
SELECT a.ROLE_NAME AS role_name, a.ROLE_HOST AS role_host
FROM INFORMATION_SCHEMA.APPLICABLE_ROLES a
--# end
