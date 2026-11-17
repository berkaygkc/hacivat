CREATE TYPE mood AS ENUM (
    'sad',
    'ok',
    'it''s fine'
);

COMMENT ON TYPE mood IS 'How it went';

CREATE TYPE floatrange AS RANGE (
    subtype = double precision
);

CREATE DOMAIN posint AS integer DEFAULT 1 NOT NULL;

CREATE TYPE address AS (
    street text,
    city text COLLATE "C"
);
