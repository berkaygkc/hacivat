-- FX-SPLIT-PG-07  psql meta-commands mixed with SQL
\set ON_ERROR_STOP on
SELECT 1;
\echo done
SELECT 2;
