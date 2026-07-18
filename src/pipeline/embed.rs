use std::sync::Arc;
use anyhow::Result;
use rusqlite::params;
use tokio::sync::mpsc;

use crate::config::{InferenceConfig, KBConfig, ProcessingConfig};
use crate::db::DbConn;
use crate::db::schema::*;
use crate::embed::EmbedEngine;

/// Start the embedding background queue.
/// `doc_id` messages arrive whenever a doc finishes parsing.
pub fn start_embed_queue(
    config: Arc<KBConfig>,
    db: Arc<tokio::sync::Mutex<DbConn>>,
    embed_engine: Option<Arc<EmbedEngine>>,
) -> mpsc::Sender<i64> {
    let (tx, mut rx) = mpsc::channel::<i64>(256);

    tokio::spawn(async move {
        while let Some(doc_id) = rx.recv().await {
            let Some(ref engine) = embed_engine else { continue };
            let batch_size = config.processing.embed_batch_size;

            if let Err(e) = process_embed(doc_id, &db, engine, batch_size).await {
                tracing::error!("embed failed for doc {}: {:#}", doc_id, e);
            }
        }
    });

    tx
}

async fn process_embed(
    doc_id: i64,
    db: &Arc<tokio::sync::Mutex<DbConn>>,
    engine: &EmbedEngine,
    batch_size: usize,
) -> Result<()> {
    loop {
        // Fetch a batch of pending chunks for this doc
        let pending: Vec<(i64, String)> = {
            let guard = db.lock().await;
            let mut stmt = guard.conn.prepare(
                "SELECT chunk_id, text FROM chunks
                 WHERE doc_id=?1 AND embed_status=0
                 ORDER BY chunk_seq LIMIT ?2",
            )?;
            stmt.query_map(params![doc_id, batch_size as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        if pending.is_empty() {
            break;
        }

        let texts: Vec<&str> = pending.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = match engine.embed_passages(&texts) {
            Ok(e) => e,
            Err(e) => {
                // Mark batch as failed
                let guard = db.lock().await;
                for (chunk_id, _) in &pending {
                    guard.conn.execute(
                        "UPDATE chunks SET embed_status=2 WHERE chunk_id=?1",
                        params![chunk_id],
                    )?;
                }
                return Err(e);
            }
        };

        // Write vectors into chunks_vec and mark done
        let guard = db.lock().await;
        for ((chunk_id, _), embedding) in pending.iter().zip(embeddings.iter()) {
            // Get doc_type for this chunk
            let doc_type: String = guard.conn.query_row(
                "SELECT d.doc_type FROM documents d JOIN chunks c ON c.doc_id=d.doc_id WHERE c.chunk_id=?1",
                params![chunk_id],
                |row| row.get(0),
            )?;

            let model_tag = get_model_tag(&guard)?;

            // Serialize embedding as JSON for sqlite-vec
            let emb_json = serde_json::to_string(embedding)?;
            guard.conn.execute(
                "INSERT OR REPLACE INTO chunks_vec(chunk_id, embedding, doc_type, model_tag)
                 VALUES (?1, ?2, ?3, ?4)",
                params![chunk_id, emb_json, doc_type, model_tag],
            )?;

            guard.conn.execute(
                "UPDATE chunks SET embed_status=1 WHERE chunk_id=?1",
                params![chunk_id],
            )?;
        }

        // Check if all chunks for this doc are done
        let pending_count: i64 = guard.conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE doc_id=?1 AND embed_status=0",
            params![doc_id],
            |row| row.get(0),
        )?;

        if pending_count == 0 {
            guard.conn.execute(
                "UPDATE documents SET status='indexed', updated_at=?1 WHERE doc_id=?2",
                params![now_ms(), doc_id],
            )?;
        }
    }

    Ok(())
}

fn get_model_tag(conn: &DbConn) -> Result<String> {
    let tag: Option<String> = conn.conn.query_row(
        "SELECT value FROM kb_meta WHERE key='model_tag'",
        [],
        |row| row.get(0),
    ).optional()?;
    Ok(tag.unwrap_or_default())
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

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
