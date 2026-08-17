-- This is the only bridge from the historical PRAGMA user_version migrations.
-- A legacy database executes it only after its actual schema matches the
-- canonical v19 manifest and passes integrity and foreign-key checks.
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK(version > 0),
    name TEXT NOT NULL UNIQUE,
    checksum TEXT NOT NULL,
    schema_fingerprint TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    app_build TEXT NOT NULL
);
