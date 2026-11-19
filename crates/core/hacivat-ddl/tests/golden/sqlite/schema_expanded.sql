CREATE TABLE authors (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE
);

CREATE TABLE notes (
  id INTEGER NOT NULL,
  author_id INTEGER REFERENCES authors(id) ON DELETE SET NULL,
  body TEXT CHECK (length(body) < 1000),
  words INTEGER GENERATED ALWAYS AS (length(body) - length(replace(body, ' ', '')) + 1) VIRTUAL,
  PRIMARY KEY (id)
) STRICT, WITHOUT ROWID;

CREATE INDEX notes_author_idx ON notes (author_id) WHERE author_id IS NOT NULL;

CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
  UPDATE authors SET name = name WHERE id = NEW.author_id;
END;

CREATE VIEW v_notes AS SELECT id, body FROM notes;
