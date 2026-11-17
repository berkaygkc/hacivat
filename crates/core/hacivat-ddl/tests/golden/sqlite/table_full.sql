CREATE TABLE authors (
    id INTEGER,
    name TEXT COLLATE NOCASE NOT NULL,
    PRIMARY KEY (id)
);

CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    body TEXT NOT NULL,
    author_id INTEGER,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    weight REAL DEFAULT 1.5,
    UNIQUE (body, author_id),
    FOREIGN KEY (author_id) REFERENCES authors (id) ON DELETE SET NULL,
    CONSTRAINT body_not_empty CHECK (length(body) > 0)
);

CREATE INDEX notes_author ON notes (author_id, created_at DESC) WHERE author_id IS NOT NULL;

CREATE INDEX notes_lower_body ON notes ((lower(body)));

CREATE TRIGGER notes_touch AFTER UPDATE ON notes BEGIN
  UPDATE notes SET created_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE VIEW v_notes AS SELECT n.id, a.name FROM notes n JOIN authors a ON a.id = n.author_id;
