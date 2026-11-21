CREATE TABLE app.people (
    id bigint NOT NULL AUTO_INCREMENT,
    name varchar(80) NOT NULL DEFAULT '?' COMMENT 'Display name',
    customer_id bigint,
    PRIMARY KEY (id),
    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES crm.customers (id) ON DELETE CASCADE,
    CONSTRAINT people_name_ck CHECK (name <> '')
) ENGINE=InnoDB COMMENT='People';

CREATE UNIQUE INDEX people_name_idx ON app.people (name DESC);
