CREATE TABLE IF NOT EXISTS pages (
    path TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    source TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS links (
    source TEXT NOT NULL REFERENCES pages(path) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    target TEXT NOT NULL,
    resolved TEXT,
    heading TEXT,
    alias TEXT,
    embedded INTEGER NOT NULL CHECK(embedded IN (0, 1)),
    PRIMARY KEY(source, position)
);
CREATE INDEX IF NOT EXISTS links_by_target ON links(resolved);
CREATE TABLE IF NOT EXISTS properties (
    page TEXT NOT NULL REFERENCES pages(path) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value_yaml TEXT NOT NULL,
    PRIMARY KEY(page, name)
);
CREATE TABLE IF NOT EXISTS cache_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    ready INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO cache_state(singleton, ready) VALUES (1, 0);
