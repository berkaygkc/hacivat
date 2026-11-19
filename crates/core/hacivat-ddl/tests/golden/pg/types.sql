CREATE TYPE mood AS ENUM (
    'sad',
    'ok',
    'it''s fine'
);

COMMENT ON TYPE mood IS 'How it went';

ALTER TYPE mood OWNER TO app_owner;

CREATE TYPE floatrange AS RANGE (
    subtype = double precision
);

ALTER TYPE floatrange OWNER TO app_owner;

CREATE DOMAIN posint AS integer
    DEFAULT 1
    NOT NULL
    CHECK ((VALUE > 0));

ALTER DOMAIN posint OWNER TO app_owner;

CREATE TYPE address AS (
    street text,
    city character varying(64)
);

ALTER TYPE address OWNER TO app_owner;
