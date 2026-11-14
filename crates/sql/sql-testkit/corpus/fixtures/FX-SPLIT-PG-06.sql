-- FX-SPLIT-PG-06  transaction keywords at top level
BEGIN;
UPDATE customers SET city = city WHERE id = 1;
COMMIT;
