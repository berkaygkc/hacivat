-- FX-SPLIT-PG-03  SQL-standard body (PG14+) with semicolons inside BEGIN ATOMIC
CREATE OR REPLACE FUNCTION hv_g(a int) RETURNS int LANGUAGE sql
BEGIN ATOMIC
  SELECT a + 1;
END;
SELECT hv_g(1);
