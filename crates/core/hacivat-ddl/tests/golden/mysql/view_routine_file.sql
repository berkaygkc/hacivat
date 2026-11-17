DELIMITER ;;
CREATE DEFINER=`root`@`%` PROCEDURE `close_order`(IN p_id BIGINT)
    COMMENT 'Closes one order'
BEGIN
  UPDATE orders SET status = 'paid' WHERE id = p_id;
  SELECT ROW_COUNT();
END;;
DELIMITER ;

CREATE DEFINER=`app`@`localhost` FUNCTION `tax`(amount DECIMAL(10,2)) RETURNS decimal(10,2)
    DETERMINISTIC
RETURN amount * 0.18;

CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id`,`orders`.`status` AS `status` from `orders` where (`orders`.`status` <> 'paid');
