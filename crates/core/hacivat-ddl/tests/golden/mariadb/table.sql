CREATE TABLE orders (
    `id` bigint unsigned NOT NULL AUTO_INCREMENT,
    customer_id int NOT NULL,
    `code` varchar(32) COLLATE utf8mb4_bin NOT NULL,
    `status` enum('new','done','') NOT NULL DEFAULT 'new',
    amount decimal(12,2) NOT NULL DEFAULT '0.00',
    total decimal(14,2) GENERATED ALWAYS AS ((`amount` * 1.2)) VIRTUAL,
    updated_at timestamp NULL DEFAULT current_timestamp() ON UPDATE current_timestamp(),
    note text COMMENT 'Approver''s path: C:\\orders',
    secret int INVISIBLE,
    PRIMARY KEY (`id`),
    UNIQUE KEY code_uq (`code`),
    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES customers (`id`) ON DELETE CASCADE,
    CHECK ((`amount` >= 0))
) ENGINE=InnoDB COLLATE=utf8mb4_uca1400_ai_ci ROW_FORMAT=DYNAMIC COMMENT='Orders';

CREATE UNIQUE INDEX note_uq ON orders (note(20));

CREATE INDEX idx_status ON orders (`status`);

CREATE INDEX idx_customer_status ON orders (customer_id, `status` DESC) COMMENT 'Per-customer queue';

CREATE INDEX idx_note_prefix ON orders (note(10));

CREATE FULLTEXT INDEX ft_note ON orders (note);

CREATE INDEX idx_lower_code ON orders ((lower(`code`)));

CREATE INDEX idx_amount_hidden ON orders (amount) IGNORED;

CREATE DEFINER=`root`@`%` TRIGGER `orders_bi` BEFORE INSERT ON `orders` FOR EACH ROW SET NEW.code = UPPER(NEW.code);
