CREATE DEFINER=`app`@`%` FUNCTION `f_tax`(x DECIMAL(12,2)) RETURNS decimal(12,2)
    DETERMINISTIC
RETURN x * 1.2;

CREATE DEFINER=`app`@`%` PROCEDURE `p_close`(IN p_id BIGINT)
BEGIN
  UPDATE orders SET status = 'done' WHERE id = p_id;
END;

CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id` from `orders` where (`orders`.`status` <> 'done');
