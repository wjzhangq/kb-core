use std::path::Path;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc;
use rusqlite::params;

use crate::config::{KBConfig, InferenceConfig, RemoteParseUnavailablePolicy};
use crate::db::DbConn;
use crate::db::schema::*;
use crate::parse::{local, remote, BlockType, Okf, OkfBlock, ParsedBy, ParsedChunk};
use crate::tantivy_idx::{writer as tw, TantivyIndex};

/// Doc ID dispatcher — parse loop processes one doc at a time from the channel.
pub fn start_parse_queue(
    config: Arc<KBConfig>,
    db: Arc<tokio::sync::Mutex<DbConn>>,
    tantivy: Arc<TantivyIndex>,
    embed_tx: mpsc::Sender<i64>,
    max_concurrency: usize,
) -> mpsc::Sender<i64> {
    let (tx, mut rx) = mpsc::channel::<i64>(256);

    tokio::spawn(async move {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrency));

        while let Some(doc_id) = rx.recv().await {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let cfg = config.clone();
            let db = db.clone();
            let tantivy = tantivy.clone();
            let embed_tx = embed_tx.clone();

            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = process_doc(doc_id, &cfg, db, tantivy, embed_tx).await {
                    tracing::error!("parse failed for doc {}: {:#}", doc_id, e);
                    // Mark as parse_failed
                    // (best effort — if DB fails here we can't do much)
                }
            });
        }
    });

    tx
}

async fn process_doc(
    doc_id: i64,
    config: &KBConfig,
    db: Arc<tokio::sync::Mutex<DbConn>>,
    tantivy: Arc<TantivyIndex>,
    embed_tx: mpsc::Sender<i64>,
) -> Result<()> {
    // Fetch doc info
    let (path, title, doc_type) = {
        let guard = db.lock().await;
        let (path, title, dt): (String, Option<String>, String) = guard.conn.query_row(
            "SELECT path, title, doc_type FROM documents WHERE doc_id = ?1",
            params![doc_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        // Mark as 'parsing'
        guard.conn.execute(
            "UPDATE documents SET status='parsing', updated_at=?1 WHERE doc_id=?2",
            params![now_ms(), doc_id],
        )?;
        (path, title, dt)
    };

    let file_path = std::path::PathBuf::from(&path);

    // Determine if this doc should go to local or remote parser
    let okf_result = try_parse(&file_path, doc_id, &doc_type, config).await;

    match okf_result {
        Err(e) => {
            let err_str = format!("{:#}", e);
            let guard = db.lock().await;
            guard.conn.execute(
                "UPDATE documents SET status='parse_failed', error=?1, updated_at=?2 WHERE doc_id=?3",
                params![err_str, now_ms(), doc_id],
            )?;
            return Err(e);
        }
        Ok(okf) => {
            let parsed_by = okf.parsed_by.as_str().to_string();
            // Derive linear text and blocks, then chunk
            let (lin_text, lin_blocks) = build_linear_text(&okf.blocks);
            let chunks = chunk_text(&lin_text, &config.processing);

            let guard = db.lock().await;

            // Store blocks
            for b in &okf.blocks {
                guard.conn.execute(
                    "INSERT OR REPLACE INTO blocks(doc_id, block_id, type, page, bbox, from_image, lin_start, lin_end)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        doc_id,
                        b.block_id as i64,
                        b.block_type.as_str(),
                        b.page.map(|p| p as i64),
                        b.bbox.map(|bbox| serde_json::to_string(&bbox).unwrap()),
                        b.from_image as i64,
                        lin_blocks[b.block_id as usize].0,
                        lin_blocks[b.block_id as usize].1,
                    ],
                )?;
            }

            // Store chunks + add to tantivy
            let title_str = title.as_deref().unwrap_or("");
            for chunk in &chunks {
                let chunk_id: i64 = guard.conn.query_row(
                    "INSERT INTO chunks(doc_id, chunk_seq, text, char_start, char_end, token_count, truncated, embed_status)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,0) RETURNING chunk_id",
                    params![doc_id, chunk.chunk_seq, chunk.text, chunk.char_start, chunk.char_end, chunk.token_count, chunk.truncated as i64],
                    |row| row.get(0),
                )?;

                tw::add_chunk(&tantivy, chunk_id as u64, doc_id as u64, &doc_type, &chunk.text, title_str, &path)?;
            }

            tw::commit(&tantivy)?;

            // Mark as 'parsed', set parsed_by
            guard.conn.execute(
                "UPDATE documents SET status='parsed', parsed_by=?1, updated_at=?2 WHERE doc_id=?3",
                params![parsed_by, now_ms(), doc_id],
            )?;

            // Enqueue for embedding (if not bm25-only)
            if !matches!(config.inference, InferenceConfig::Bm25Only) {
                let _ = embed_tx.send(doc_id).await;
            } else {
                // Mark all chunks as skipped
                guard.conn.execute(
                    "UPDATE chunks SET embed_status=3 WHERE doc_id=?1",
                    params![doc_id],
                )?;
                // Mark document as indexed immediately
                guard.conn.execute(
                    "UPDATE documents SET status='indexed', updated_at=?1 WHERE doc_id=?2",
                    params![now_ms(), doc_id],
                )?;
            }
        }
    }

    Ok(())
}

async fn try_parse(
    path: &std::path::Path,
    doc_id: i64,
    doc_type: &str,
    config: &KBConfig,
) -> Result<Okf> {
    let processing = &config.processing;

    // Image-type documents (or PDF without text layer) need remote
    let needs_remote = matches!(doc_type, "image" | "pptx")
        || (doc_type == "pdf");

    if needs_remote {
        // Check if remote parse is configured
        let remote_config = match &config.inference {
            InferenceConfig::LocalFirst { parse: Some(rpc), .. } => Some(rpc.clone()),
            InferenceConfig::Remote { parse: Some(rpc), .. } => Some(rpc.clone()),
            _ => None,
        };

        if let Some(rpc) = remote_config {
            let parser = crate::parse::remote::RemoteParser::new(rpc.clone());
            match parser.parse_file(path, doc_id).await {
                Ok(okf) => return Ok(okf),
                Err(e) => {
                    match rpc.on_remote_parse_unavailable {
                        RemoteParseUnavailablePolicy::Skip => {
                            return Err(anyhow::anyhow!("remote parse unavailable, skipping: {}", e));
                        }
                        RemoteParseUnavailablePolicy::TextOnly => {
                            // Fall through to local
                        }
                        RemoteParseUnavailablePolicy::Wait => {
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    // Local parse
    local::extract_local(path, doc_id, processing)
}

/// Build linear text from okf blocks (joined with "\n\n").
/// Returns (linear_text, Vec<(lin_start, lin_end)> per block).
fn build_linear_text(blocks: &[OkfBlock]) -> (String, Vec<(i64, i64)>) {
    let mut result = String::new();
    let mut spans = Vec::with_capacity(blocks.len());

    for (i, block) in blocks.iter().enumerate() {
        let start = result.len() as i64;
        result.push_str(&block.text);
        let end = result.len() as i64;
        spans.push((start, end));
        if i + 1 < blocks.len() {
            result.push_str("\n\n");
        }
    }

    (result, spans)
}

/// Split linear text into chunks respecting token limits.
fn chunk_text(text: &str, cfg: &crate::config::ProcessingConfig) -> Vec<ParsedChunk> {
    let max_chars = cfg.chunk_max_tokens * 4; // ~4 chars per token average
    let mut chunks = Vec::new();
    let mut seq: i64 = 0;

    if text.is_empty() {
        return chunks;
    }

    let mut start = 0usize;
    while start < text.len() {
        let end = (start + max_chars).min(text.len());

        // Try to break at a paragraph boundary first
        let actual_end = if end < text.len() {
            text[start..end].rfind("\n\n")
                .map(|p| start + p + 2)
                .or_else(|| text[start..end].rfind('\n').map(|p| start + p + 1))
                .or_else(|| text[start..end].rfind(' ').map(|p| start + p + 1))
                .unwrap_or(end)
        } else {
            end
        };

        let chunk_text = text[start..actual_end].trim().to_string();
        if !chunk_text.is_empty() {
            let token_count = chunk_text.len() / 4;
            let truncated = token_count > cfg.chunk_max_tokens;
            chunks.push(ParsedChunk {
                chunk_seq: seq,
                text: chunk_text,
                char_start: start as i64,
                char_end: actual_end as i64,
                token_count: token_count as i64,
                truncated,
            });
            seq += 1;
        }

        if actual_end <= start { break; }
        start = actual_end;
    }

    chunks
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
