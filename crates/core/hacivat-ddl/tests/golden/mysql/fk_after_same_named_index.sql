CREATE TABLE `lines` (
    order_id bigint unsigned NOT NULL,
    no int NOT NULL
);

CREATE INDEX lines_order ON `lines` (order_id, no);

ALTER TABLE `lines` ADD CONSTRAINT lines_order FOREIGN KEY (order_id) REFERENCES orders (id);
