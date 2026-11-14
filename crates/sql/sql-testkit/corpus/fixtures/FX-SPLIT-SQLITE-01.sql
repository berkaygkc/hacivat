-- FX-SPLIT-SQLITE-01  trigger with BEGIN...END
CREATE TRIGGER hv_tr AFTER INSERT ON orders
BEGIN
  INSERT INTO audit_log(actor, message) VALUES ('trg', 'order;added');
END;
SELECT 1;
