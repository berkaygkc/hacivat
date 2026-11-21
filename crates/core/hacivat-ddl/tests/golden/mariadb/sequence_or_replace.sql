CREATE SEQUENCE seq_invoice
    START WITH 1000;

DELIMITER ;;
CREATE OR REPLACE PROCEDURE `p_close`(IN p_id BIGINT)
BEGIN
  UPDATE orders SET status = 'done' WHERE id = p_id;
END;;
DELIMITER ;

CREATE OR REPLACE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id` from `orders` where `orders`.`status` <> 'done';
