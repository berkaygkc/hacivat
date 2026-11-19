CREATE INDEX orders_code_hash ON orders USING hash (code);

CREATE INDEX orders_lower_note_idx ON shop.orders USING btree (lower(note) text_pattern_ops);

CREATE INDEX orders_open_idx ON shop.orders USING btree (created_at DESC NULLS LAST) INCLUDE (id) WHERE (status <> 'done'::text);

COMMENT ON INDEX orders_open_idx IS 'Open orders, newest first';
