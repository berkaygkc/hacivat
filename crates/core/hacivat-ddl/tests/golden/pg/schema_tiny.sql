CREATE SCHEMA tiny;

ALTER SCHEMA tiny OWNER TO tiny_owner;

CREATE TABLE tiny.parent (
    id integer NOT NULL,
    PRIMARY KEY (id)
);

ALTER TABLE tiny.parent OWNER TO tiny_owner;

CREATE TABLE tiny.child (
    id integer NOT NULL,
    parent_id integer NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (parent_id) REFERENCES tiny.parent (id)
);
