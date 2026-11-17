CREATE TABLE aux.kv (
    k TEXT NOT NULL,
    v BLOB,
    PRIMARY KEY (k)
);

CREATE INDEX aux.kv_v ON kv (v);

CREATE TRIGGER aux.kv_guard BEFORE DELETE ON kv BEGIN
  SELECT RAISE(ABORT, 'read-only');
END;
