CREATE OR REPLACE FUNCTION shop.add_tax(amount numeric, rate numeric)
 RETURNS numeric
 LANGUAGE sql
 IMMUTABLE
BEGIN ATOMIC
 SELECT (add_tax.amount * ((1)::numeric + add_tax.rate));
END;

ALTER FUNCTION add_tax(numeric, numeric) OWNER TO app_owner;

CREATE OR REPLACE FUNCTION shop.touch()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
  NEW.updated_at := now(); -- keep it fresh
  RETURN NEW;
END
$function$;

COMMENT ON FUNCTION touch() IS 'Sets updated_at';

ALTER FUNCTION touch() OWNER TO app_owner;

CREATE TABLE items (
    id integer NOT NULL,
    updated_at timestamp with time zone
);

CREATE OR REPLACE TRIGGER items_touch BEFORE UPDATE ON shop.items FOR EACH ROW EXECUTE FUNCTION shop.touch();
