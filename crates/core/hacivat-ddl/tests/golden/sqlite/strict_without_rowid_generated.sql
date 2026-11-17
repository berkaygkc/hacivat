CREATE TABLE prices (
    sku TEXT NOT NULL,
    qty INTEGER NOT NULL,
    price REAL NOT NULL,
    total REAL GENERATED ALWAYS AS (qty * price) STORED,
    label TEXT GENERATED ALWAYS AS (upper(sku)) VIRTUAL,
    meta ANY,
    PRIMARY KEY (sku)
) STRICT, WITHOUT ROWID;
