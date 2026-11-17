CREATE TABLE orders (
    id bigint GENERATED ALWAYS AS IDENTITY NOT NULL,
    customer_id bigint NOT NULL,
    code character varying(32) COLLATE "C" NOT NULL,
    status text DEFAULT 'new'::text NOT NULL,
    amount numeric(12,2) DEFAULT 0 NOT NULL,
    total numeric GENERATED ALWAYS AS ((amount * 1.2)) STORED,
    note text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (id),
    CONSTRAINT orders_code_uq UNIQUE (code),
    CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES shop.customers (id) ON DELETE CASCADE,
    CHECK ((amount >= (0)::numeric))
);

COMMENT ON TABLE orders IS 'Customer orders';

COMMENT ON COLUMN orders.note IS 'PM''s approver of record';

CREATE INDEX orders_status_idx ON orders (status);

CREATE INDEX orders_open_idx ON orders (created_at DESC NULLS LAST) INCLUDE (id) WHERE (status <> 'done'::text);

COMMENT ON INDEX orders_open_idx IS 'Open orders, newest first';

CREATE INDEX orders_lower_note_idx ON orders ((lower(note)));

CREATE INDEX orders_code_hash ON orders USING hash (code);

CREATE TRIGGER orders_audit AFTER INSERT OR UPDATE ON shop.orders FOR EACH ROW EXECUTE FUNCTION shop.audit_row();

ALTER TABLE orders OWNER TO app_owner;

GRANT SELECT ON TABLE orders TO PUBLIC;

GRANT SELECT ON TABLE orders TO app_reader;

GRANT INSERT, UPDATE ON TABLE orders TO app_writer WITH GRANT OPTION;

GRANT UPDATE (note, status) ON TABLE orders TO support;
