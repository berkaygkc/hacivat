CREATE UNLOGGED TABLE IF NOT EXISTS logs (
    line text
)
WITH (fillfactor=70, autovacuum_enabled=false);
