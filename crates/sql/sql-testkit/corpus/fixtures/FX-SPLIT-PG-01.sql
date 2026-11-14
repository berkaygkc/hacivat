-- FX-SPLIT-PG-01  dollar-quoted DO block with semicolons and a string containing ;
DO $$
BEGIN
  RAISE NOTICE 'a;b';
  PERFORM 1;
END
$$;
SELECT 1;
