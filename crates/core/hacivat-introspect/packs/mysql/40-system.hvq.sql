-- Hacivat query pack: MySQL and MariaDB, file 40-system (HV-INTRO-04, R-M4-32).
-- Contents: system-mode sections for the minicat snapshots (R-M4-44). Sections are written by Task 17 from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
-- ## m4-t17-mysql (M4 Task 17): `Mode::System` reads of `information_schema`, `mysql`,
-- `performance_schema` and `sys` (R-M4-44). Only what is stable across fresh containers of
-- one image (R-M4-17): names, kinds, types and positions; no times, sizes, row counts,
-- definers or comments. Variable names only (`ListSettings`): never values, never comments.

--- name: SystemRelations ---
--- returns: table_name text, table_type text ---
SELECT t.TABLE_NAME AS table_name, t.TABLE_TYPE AS table_type
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_SCHEMA = :schema

--- name: SystemColumns ---
--- returns: table_name text, column_name text, ordinal_position u32, column_type text, is_nullable text ---
SELECT c.TABLE_NAME AS table_name, c.COLUMN_NAME AS column_name,
  c.ORDINAL_POSITION AS ordinal_position, c.COLUMN_TYPE AS column_type,
  c.IS_NULLABLE AS is_nullable
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_SCHEMA = :schema

--- name: SystemRoutines ---
--- returns: specific_name text, routine_name text, routine_type text, dtd_identifier text ---
SELECT r.SPECIFIC_NAME AS specific_name, r.ROUTINE_NAME AS routine_name,
  r.ROUTINE_TYPE AS routine_type, r.DTD_IDENTIFIER AS dtd_identifier
FROM INFORMATION_SCHEMA.ROUTINES r
WHERE r.ROUTINE_SCHEMA = :schema

--- name: SystemParameters ---
--- returns: specific_name text, routine_type text, ordinal_position u32, parameter_mode text, parameter_name text, dtd_identifier text ---
SELECT p.SPECIFIC_NAME AS specific_name, p.ROUTINE_TYPE AS routine_type,
  p.ORDINAL_POSITION AS ordinal_position, p.PARAMETER_MODE AS parameter_mode,
  p.PARAMETER_NAME AS parameter_name, p.DTD_IDENTIFIER AS dtd_identifier
FROM INFORMATION_SCHEMA.PARAMETERS p
WHERE p.SPECIFIC_SCHEMA = :schema AND p.ORDINAL_POSITION > 0

--- name: ListSettings ---
--- returns: variable_name text, variable_scope text, variable_type text ---
--# if flag(mariadb)
SELECT s.VARIABLE_NAME AS variable_name, s.VARIABLE_SCOPE AS variable_scope,
  s.VARIABLE_TYPE AS variable_type
FROM INFORMATION_SCHEMA.SYSTEM_VARIABLES s
--# else
SELECT v.VARIABLE_NAME AS variable_name, NULL AS variable_scope, NULL AS variable_type
FROM performance_schema.variables_info v
--# end
