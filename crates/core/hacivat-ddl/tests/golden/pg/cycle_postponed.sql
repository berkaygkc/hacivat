CREATE TABLE a (
    id integer NOT NULL,
    b_id integer,
    PRIMARY KEY (id)
);

CREATE TABLE b (
    id integer NOT NULL,
    a_id integer,
    PRIMARY KEY (id),
    CONSTRAINT b_a_fk FOREIGN KEY (a_id) REFERENCES a (id)
);

ALTER TABLE a ADD CONSTRAINT a_b_fk FOREIGN KEY (b_id) REFERENCES b (id) DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE a ADD CONSTRAINT a_b_id_check CHECK ((b_id > 0)) NOT VALID;
