CREATE OR REPLACE VIEW active_orders AS
 SELECT orders.id,
    orders.status
   FROM shop.orders
  WHERE (orders.status <> 'done'::text)
  WITH LOCAL CHECK OPTION;

COMMENT ON VIEW active_orders IS 'Open orders';

COMMENT ON COLUMN active_orders.status IS 'Never ''done''';

ALTER VIEW active_orders OWNER TO app_owner;

CREATE MATERIALIZED VIEW order_totals AS
 SELECT orders.customer_id,
    sum(orders.amount) AS total
   FROM shop.orders
  GROUP BY orders.customer_id;

CREATE INDEX order_totals_customer_idx ON order_totals (customer_id);

ALTER MATERIALIZED VIEW order_totals OWNER TO app_owner;
