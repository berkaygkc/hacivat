CREATE TABLE inline_keys (
    id integer NOT NULL,
    code text,
    customer_id bigint,
    lo integer,
    hi integer
);

ALTER TABLE inline_keys ADD CONSTRAINT pk PRIMARY KEY (id);

ALTER TABLE inline_keys ADD CONSTRAINT inline_keys_code_key UNIQUE (code);

ALTER TABLE inline_keys ADD CONSTRAINT inline_keys_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES sales.customers (id) MATCH FULL ON UPDATE RESTRICT ON DELETE SET NULL;

ALTER TABLE inline_keys ADD CONSTRAINT lo_hi UNIQUE (lo, hi);

ALTER TABLE inline_keys ADD CONSTRAINT lo_le_hi CHECK ((lo <= hi));

ALTER TABLE inline_keys ADD CONSTRAINT no_overlap EXCLUDE USING gist (int4range(lo, hi) WITH &&);
