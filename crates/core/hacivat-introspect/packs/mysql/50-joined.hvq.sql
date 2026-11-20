-- Hacivat query pack: MySQL and MariaDB, file 50-joined (HV-INTRO-04, R-M4-32).
-- Contents: the fragment read of a joined connection (R-M4-29; Task 15b).
-- Written by M4 Task 15b from the MySQL 8.0/8.4 reference manual (INFORMATION_SCHEMA TABLES
-- and COLUMNS) and the MariaDB knowledge base only. No third-party catalog SQL.
--
-- These sections run inside a transaction Hacivat owns on a job session (M7's drift check),
-- read only INFORMATION_SCHEMA, take the schema and the relation names as binds, and name no
-- function: nothing in them depends on the session's state.

--- name: JoinedTables ---
--- returns: table_name text, table_type text ---
--- joinable ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_SCHEMA = :schema AND t.TABLE_NAME IN :[names]
ORDER BY t.TABLE_NAME

--- name: JoinedColumns ---
--- returns: table_name text, column_name text, ordinal_position u32, column_type text, is_nullable text ---
--- joinable ---
SELECT c.TABLE_NAME AS table_name, c.COLUMN_NAME AS column_name,
  c.ORDINAL_POSITION AS ordinal_position, c.COLUMN_TYPE AS column_type,
  c.IS_NULLABLE AS is_nullable
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_SCHEMA = :schema AND c.TABLE_NAME IN :[names]
ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION
