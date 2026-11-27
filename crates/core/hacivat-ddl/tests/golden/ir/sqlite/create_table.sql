CREATE TABLE app.people (
    id INTEGER NOT NULL CONSTRAINT people_pk PRIMARY KEY AUTOINCREMENT,
    name varchar(80) DEFAULT '?' NOT NULL,
    CONSTRAINT people_name_ck CHECK (name <> '')
);

CREATE UNIQUE INDEX app.people_name_idx ON people (name DESC);
