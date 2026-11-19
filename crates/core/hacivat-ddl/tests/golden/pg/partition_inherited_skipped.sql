CREATE TABLE events_2026 PARTITION OF events (
    CONSTRAINT events_2026_payload_check CHECK ((payload IS NOT NULL))
) FOR VALUES FROM ('2026-01-01 00:00:00+00') TO ('2027-01-01 00:00:00+00')
WITH (fillfactor=90)
TABLESPACE fast_ssd;
