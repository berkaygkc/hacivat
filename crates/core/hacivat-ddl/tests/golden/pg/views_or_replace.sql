CREATE OR REPLACE VIEW active_orders AS
 SELECT o.id,
    o.status
   FROM shop.orders o
  WHERE (o.status <> 'done'::text)
  WITH LOCAL CHECK OPTION;

COMMENT ON VIEW active_orders IS 'Orders not done yet';

COMMENT ON COLUMN active_orders.id IS 'Order id';

ALTER VIEW active_orders OWNER TO app_owner;

CREATE MATERIALIZED VIEW order_totals AS
 SELECT orders.customer_id,
    sum(orders.amount) AS total
   FROM shop.orders
  GROUP BY orders.customer_id;

CREATE UNIQUE INDEX order_totals_customer_idx ON order_totals (customer_id);
