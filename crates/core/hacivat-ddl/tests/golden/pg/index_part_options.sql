CREATE TABLE rare_opts (
    a text,
    b integer,
    room integer,
    during tsrange,
    CONSTRAINT rare_opts_room_excl EXCLUDE USING gist (room WITH =, during WITH &&) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT rare_opts_b_check CHECK (b > 0) NO INHERIT
);

CREATE INDEX rare_opts_ab_idx ON rare_opts (a COLLATE "C" text_pattern_ops, b DESC NULLS LAST);

CREATE INDEX rare_opts_a_trgm ON rare_opts USING gin (a public.gin_trgm_ops(siglen=32));

CREATE INDEX rare_opts_b_nulls_first ON rare_opts (b NULLS FIRST);
