CREATE TABLE inline_keys (
    id integer NOT NULL CONSTRAINT pk PRIMARY KEY,
    code text UNIQUE,
    customer_id bigint REFERENCES sales.customers (id) MATCH FULL ON UPDATE RESTRICT ON DELETE SET NULL,
    lo integer,
    hi integer,
    CONSTRAINT lo_hi UNIQUE (lo, hi),
    CONSTRAINT lo_le_hi CHECK ((lo <= hi)),
    CONSTRAINT no_overlap EXCLUDE USING gist (int4range(lo, hi) WITH &&)
);
