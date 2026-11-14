-- FX-CARET-01  nested subquery and CTE (PostgreSQL)
WITH recent AS (
  SELECT id, customer_id FROM orders WHERE ordered_at > now() - interval '30 days'
)
SELECT c.full_name,
       (SELECT count(*) FROM order_items oi WHERE oi.order_id IN (SELECT id FROM recent r WHERE r.customer_id = c.id)) AS n
FROM customers c;

SELECT 2;
