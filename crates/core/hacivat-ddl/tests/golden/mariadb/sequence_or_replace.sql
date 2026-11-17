CREATE SEQUENCE invoice_seq
    START WITH 1000;

DELIMITER ;;
CREATE OR REPLACE PROCEDURE `close_order`(IN p_id BIGINT)
    COMMENT 'Closes one order'
BEGIN
  UPDATE orders SET status = 'paid' WHERE id = p_id;
  SELECT ROW_COUNT();
END;;
DELIMITER ;

CREATE OR REPLACE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id`,`orders`.`status` AS `status` from `orders` where (`orders`.`status` <> 'paid');

DELIMITER ;;
CREATE OR REPLACE TRIGGER `orders_bi` BEFORE INSERT ON `orders` FOR EACH ROW BEGIN
  IF NEW.price < 0 THEN
    SET NEW.price = 0;
  END IF;
END;;
DELIMITER ;
