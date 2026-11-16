-- FX-SPLIT-PG-05  COPY FROM STDIN with data lines (psql style)
COPY audit_log(actor, message) FROM STDIN;
sys	hello; world
sys	second
\.
SELECT count(*) FROM audit_log;
