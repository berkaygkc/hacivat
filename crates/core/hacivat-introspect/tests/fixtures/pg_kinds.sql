-- One of every object kind the PostgreSQL introspector reads (PG 13-18; M4 Task 16, spike origin 5a5ab08).
-- the schema name `hvm4_fx` with its own unique name before it runs this file.
CREATE SCHEMA hvm4_fx;
SET search_path = hvm4_fx;
COMMENT ON SCHEMA hvm4_fx IS 'fixture schema';
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
COMMENT ON TYPE mood IS 'feelings';
CREATE DOMAIN posint AS integer NOT NULL DEFAULT 1 CONSTRAINT posint_ck CHECK (VALUE > 0);
CREATE TYPE pair AS (a int, b text);
CREATE TYPE floatrange AS RANGE (subtype = float8);
CREATE TABLE parent (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code text NOT NULL UNIQUE,
    tags text[],
    grid int[][],
    m mood DEFAULT 'ok',
    p posint,
    pr pair,
    moods mood[],
    total numeric(10, 2) DEFAULT 0,
    doubled numeric GENERATED ALWAYS AS (total * 2) STORED,
    name text COLLATE "C",
    ts timestamptz(3) DEFAULT now(),
    CONSTRAINT parent_total_ck CHECK (total >= 0)
);
COMMENT ON TABLE parent IS 'Parents';
COMMENT ON COLUMN parent.code IS 'Code';
CREATE TABLE child (
    id serial PRIMARY KEY,
    parent_id bigint NOT NULL REFERENCES parent (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    a int,
    b int,
    r floatrange,
    CONSTRAINT child_ab_uq UNIQUE (b, a),
    CONSTRAINT child_r_excl EXCLUDE USING gist (r WITH &&)
);
COMMENT ON CONSTRAINT child_ab_uq ON child IS 'uq';
CREATE INDEX child_expr_idx ON child ((a + b)) INCLUDE (parent_id) WHERE a > 0;
CREATE INDEX child_desc_idx ON child (b DESC NULLS LAST, a);
COMMENT ON INDEX child_desc_idx IS 'desc';
CREATE TABLE events (id int, at date NOT NULL) PARTITION BY RANGE (at);
CREATE TABLE events_2026 PARTITION OF events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
CREATE VIEW v_parent AS SELECT id, code FROM parent WHERE total > 0;
COMMENT ON VIEW v_parent IS 'positive parents';
CREATE MATERIALIZED VIEW mv_child AS
    SELECT parent_id, count(*) AS n FROM child GROUP BY parent_id WITH NO DATA;
CREATE SEQUENCE seq_x AS integer INCREMENT 5 MINVALUE 10 MAXVALUE 1000 START 10 CACHE 2 CYCLE;
COMMENT ON SEQUENCE seq_x IS 'seq';
CREATE FUNCTION add(a int, b int) RETURNS int LANGUAGE sql IMMUTABLE STRICT AS 'SELECT a + b';
CREATE FUNCTION add(a text, b text) RETURNS text LANGUAGE sql AS 'SELECT a || b';
COMMENT ON FUNCTION add(int, int) IS 'adds';
CREATE FUNCTION series(n int) RETURNS SETOF int LANGUAGE sql STABLE AS 'SELECT generate_series(1, n)';
CREATE PROCEDURE noop(x int) LANGUAGE plpgsql AS $$ BEGIN END $$;
CREATE FUNCTION trg() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE TRIGGER child_bi BEFORE INSERT OR UPDATE OF a ON child
    FOR EACH ROW WHEN (NEW.a IS NOT NULL) EXECUTE FUNCTION trg();
COMMENT ON TRIGGER child_bi ON child IS 'trg';
CREATE AGGREGATE mysum(int) (SFUNC = int4pl, STYPE = int);
CREATE TABLE "Mixed Case" ("Ünïcode Col" int, "quote""d" text);
CREATE UNLOGGED TABLE u (a int);
RESET search_path;
