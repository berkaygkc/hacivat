-- FX-SPLIT-PG-02  named dollar tag with nested $$ inside
CREATE OR REPLACE FUNCTION hv_f() RETURNS text LANGUAGE plpgsql AS $body$
DECLARE s text := $$x;y$$;
BEGIN
  RETURN s || ';';
END
$body$;
SELECT hv_f();
