CREATE TABLE loose (
    customer_id bigint
);

ALTER TABLE loose ADD CONSTRAINT loose_customer_check CHECK ((customer_id > 0)) NOT ENFORCED;

ALTER TABLE loose ADD CONSTRAINT loose_customer_fk FOREIGN KEY (customer_id) REFERENCES shop.customers (id) NOT ENFORCED;
