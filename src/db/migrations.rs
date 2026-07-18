use anyhow::{Context, Result};
use rusqlite::Connection;

pub const CURRENT_VERSION: i64 = 5;

const MIGRATION_001: &str = "
CREATE TABLE IF NOT EXISTS documents (
    doc_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT NOT NULL UNIQUE,
    title       TEXT,
    doc_type    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending_parse',
    parsed_by   TEXT,
    error       TEXT,
    added_at    INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    chunk_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id       INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
    chunk_seq    INTEGER NOT NULL,
    text         TEXT NOT NULL,
    char_start   INTEGER NOT NULL DEFAULT 0,
    char_end     INTEGER NOT NULL DEFAULT 0,
    token_count  INTEGER NOT NULL DEFAULT 0,
    truncated    INTEGER NOT NULL DEFAULT 0,
    embed_status INTEGER NOT NULL DEFAULT 0,
    UNIQUE(doc_id, chunk_seq)
);

CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);
CREATE INDEX IF NOT EXISTS idx_chunks_embed_pending ON chunks(embed_status) WHERE embed_status = 0;

CREATE TABLE IF NOT EXISTS kb_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

const MIGRATION_002: &str = "
INSERT OR IGNORE INTO kb_meta(key, value) VALUES ('schema_version', '2');
INSERT OR IGNORE INTO kb_meta(key, value) VALUES ('embedding_model', 'multilingual-e5-small');
INSERT OR IGNORE INTO kb_meta(key, value) VALUES ('embedding_dim', '384');
INSERT OR IGNORE INTO kb_meta(key, value) VALUES ('embedding_quant', 'int8');
";

const MIGRATION_003: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(
    chunk_id   INTEGER PRIMARY KEY,
    embedding  float[384],
    +doc_type  TEXT,
    +model_tag TEXT
);
";

const MIGRATION_004: &str = "
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
";

const MIGRATION_005: &str = "
CREATE TABLE IF NOT EXISTS blocks (
    doc_id      INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
    block_id    INTEGER NOT NULL,
    type        TEXT NOT NULL,
    page        INTEGER,
    bbox        TEXT,
    from_image  INTEGER NOT NULL DEFAULT 0,
    lin_start   INTEGER NOT NULL,
    lin_end     INTEGER NOT NULL,
    PRIMARY KEY (doc_id, block_id)
);

CREATE INDEX IF NOT EXISTS idx_blocks_span ON blocks(doc_id, lin_start, lin_end);
";

pub fn run_migrations(conn: &Connection) -> Result<()> {
    let version = get_schema_version(conn)?;

    if version < 1 {
        conn.execute_batch(MIGRATION_001)
            .context("migration 001 failed")?;
        set_schema_version(conn, 1)?;
    }
    if version < 2 {
        conn.execute_batch(MIGRATION_002)
            .context("migration 002 failed")?;
        set_schema_version(conn, 2)?;
    }
    if version < 3 {
        // sqlite-vec extension must already be loaded
        conn.execute_batch(MIGRATION_003)
            .context("migration 003 failed (sqlite-vec not loaded?)")?;
        set_schema_version(conn, 3)?;
    }
    if version < 4 {
        conn.execute_batch(MIGRATION_004)
            .context("migration 004 failed")?;
        set_schema_version(conn, 4)?;
    }
    if version < 5 {
        conn.execute_batch(MIGRATION_005)
            .context("migration 005 failed")?;
        set_schema_version(conn, 5)?;
    }

    Ok(())
}

fn get_schema_version(conn: &Connection) -> Result<i64> {
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='kb_meta'",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    let v: Option<i64> = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM kb_meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(v.unwrap_or(0))
}

fn set_schema_version(conn: &Connection, v: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO kb_meta(key, value) VALUES ('schema_version', ?1)",
        [&v.to_string()],
    )?;
    Ok(())
}

// Extension trait for optional query_row
trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
