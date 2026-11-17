CREATE TABLE orders (
    `id` bigint unsigned NOT NULL AUTO_INCREMENT,
    customer_id bigint unsigned NOT NULL,
    `code` varchar(32) COLLATE utf8mb4_bin NOT NULL,
    `status` enum('new','paid','it''s done') NOT NULL DEFAULT 'new',
    note varchar(200) COMMENT 'it''s "quoted" and C:\\path',
    price decimal(10,2),
    total decimal(12,2) GENERATED ALWAYS AS ((`price` * 1.2)) VIRTUAL,
    created_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    closed_at timestamp NULL,
    PRIMARY KEY (`id`),
    UNIQUE KEY code_uq (`code`),
    KEY fk_customer (customer_id),
    KEY idx_status (`status`) COMMENT 'for the board',
    KEY idx_note (note(10), created_at DESC),
    KEY idx_lower_code ((lower(`code`))),
    FULLTEXT KEY ft_note (note),
    KEY idx_code_hash (`code`, `status`) USING HASH,
    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES crm.customers (`id`) ON DELETE CASCADE,
    CONSTRAINT chk_price CHECK ((`price` >= 0))
) ENGINE=InnoDB COLLATE=utf8mb4_0900_ai_ci COMMENT='Orders';

CREATE DEFINER=`root`@`%` TRIGGER `orders_bi` BEFORE INSERT ON `orders` FOR EACH ROW BEGIN
  IF NEW.price < 0 THEN
    SET NEW.price = 0;
  END IF;
END;
