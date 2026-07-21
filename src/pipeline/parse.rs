use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc;
use rusqlite::params;

use crate::config::{KBConfig, InferenceConfig, RemoteParseUnavailablePolicy};
use crate::db::DbConn;
use crate::parse::{local, Okf, OkfBlock, ParsedChunk};
use crate::tantivy_idx::{writer as tw, TantivyIndex};

/// Doc ID dispatcher — parse loop processes one doc at a time from the channel.
pub fn start_parse_queue(
    config: Arc<KBConfig>,
    db: Arc<tokio::sync::Mutex<DbConn>>,
    tantivy: Arc<TantivyIndex>,
    embed_tx: mpsc::Sender<i64>,
    max_concurrency: usize,
) -> (mpsc::Sender<i64>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<i64>(256);

    let handle = tokio::spawn(async move {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrency));

        while let Some(doc_id) = rx.recv().await {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let cfg = config.clone();
            let db = db.clone();
            let tantivy = tantivy.clone();
            let embed_tx = embed_tx.clone();

            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = process_doc(doc_id, &cfg, db.clone(), tantivy, embed_tx).await {
                    tracing::error!("parse failed for doc {}: {:#}", doc_id, e);
                    // Best-effort fallback: ensure status is not left as 'parsing'.
                    // Uses AND status='parsing' to avoid overwriting a more-detailed
                    // parse_failed already written by process_doc itself (FR-003).
                    let guard = db.lock().await;
                    let _ = guard.conn.execute(
                        "UPDATE documents SET status='parse_failed', updated_at=?1 \
                         WHERE doc_id=?2 AND status='parsing'",
                        params![now_ms(), doc_id],
                    );
                }
            });
        }

        // Channel closed (all senders dropped by close()). Drain in-flight
        // process_doc tasks before returning: acquiring every permit succeeds
        // only once each detached task has finished and released its db /
        // tantivy Arc clone. Without this, close() could drop the DbConn while
        // a parse task still holds a clone, leaving the kb.db handle open.
        let _ = semaphore.acquire_many(max_concurrency as u32).await;
        // Drop the last embed_tx clone so the embed queue observes channel close.
        drop(embed_tx);
    });

    (tx, handle)
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

            // Store blocks (FR-002): use HashMap lookup; skip + warn on missing block_id
            for b in &okf.blocks {
                let Some(&(lin_start, lin_end)) = lin_blocks.get(&b.block_id) else {
                    tracing::warn!(
                        "doc {}: block_id {} not in lin_blocks, skipping",
                        doc_id, b.block_id
                    );
                    continue;
                };
                guard.conn.execute(
                    "INSERT OR REPLACE INTO blocks(doc_id, block_id, type, page, bbox, from_image, lin_start, lin_end, description)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        doc_id,
                        b.block_id as i64,
                        b.block_type.as_str(),
                        b.page.map(|p| p as i64),
                        b.bbox.map(|bbox| serde_json::to_string(&bbox).unwrap()),
                        b.from_image as i64,
                        lin_start,
                        lin_end,
                        b.description.as_deref(),
                    ],
                )?;
            }

            // Flatten outline nodes as outline_heading blocks (appended after content blocks)
            // so they participate in BM25 and vector search.
            if let Some(outline) = &okf.outline {
                let mut next_block_id = okf.blocks.iter().map(|b| b.block_id).max().map(|m| m + 1).unwrap_or(0);
                let mut outline_queue: Vec<&crate::parse::OutlineNode> = outline.iter().collect();
                let mut lin_pos = if let Some(&(_, end)) = lin_blocks.values().max_by_key(|(_, e)| e) {
                    end + 2 // after the last block's "\n\n"
                } else {
                    0
                };
                while !outline_queue.is_empty() {
                    let batch: Vec<&crate::parse::OutlineNode> = outline_queue.drain(..).collect();
                    for node in batch {
                        let title = &node.title;
                        let lin_start = lin_pos;
                        let lin_end = lin_pos + title.len() as i64;
                        guard.conn.execute(
                            "INSERT OR IGNORE INTO blocks(doc_id, block_id, type, page, bbox, from_image, lin_start, lin_end, description)
                             VALUES (?1,?2,?3,?4,NULL,0,?5,?6,NULL)",
                            params![
                                doc_id,
                                next_block_id as i64,
                                crate::parse::BlockType::OutlineHeading.as_str(),
                                node.page.map(|p| p as i64),
                                lin_start,
                                lin_end,
                            ],
                        )?;
                        next_block_id += 1;
                        lin_pos = lin_end + 2;
                        outline_queue.extend(node.children.iter());
                    }
                }
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
/// Returns (linear_text, HashMap<block_id, (lin_start, lin_end)>).
/// Keying by block_id (FR-002) avoids out-of-bounds panics when block_ids
/// are non-contiguous or non-zero-based.
/// For image_ocr / image_caption blocks, description is appended after text
/// so both OCR content and visual description enter the same BM25/vector index.
pub fn build_linear_text(blocks: &[OkfBlock]) -> (String, HashMap<u32, (i64, i64)>) {
    let mut result = String::new();
    let mut spans: HashMap<u32, (i64, i64)> = HashMap::with_capacity(blocks.len());

    for (i, block) in blocks.iter().enumerate() {
        let start = result.len() as i64;
        // For image blocks, append the model-generated description after OCR text.
        let content = match (&block.block_type, &block.description) {
            (crate::parse::BlockType::ImageOcr | crate::parse::BlockType::ImageCaption, Some(desc)) if !desc.is_empty() => {
                if block.text.is_empty() {
                    desc.clone()
                } else {
                    format!("{}\n{}", block.text, desc)
                }
            }
            _ => block.text.clone(),
        };
        result.push_str(&content);
        let end = result.len() as i64;
        spans.insert(block.block_id, (start, end));
        if i + 1 < blocks.len() {
            result.push_str("\n\n");
        }
    }

    (result, spans)
}

/// Largest char-boundary index `<= i` in `text`.
///
/// Equivalent to the unstable-on-our-MSRV `str::floor_char_boundary` (stable in
/// Rust 1.91; our MSRV is 1.78), so CJK / multi-byte characters are never split
/// mid-codepoint.
fn floor_char_boundary(text: &str, i: usize) -> usize {
    if i >= text.len() {
        return text.len();
    }
    let mut b = i;
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Split linear text into chunks respecting token limits.
pub fn chunk_text(text: &str, cfg: &crate::config::ProcessingConfig) -> Vec<ParsedChunk> {
    let max_chars = cfg.chunk_max_tokens * 4; // ~4 chars per token average
    let mut chunks = Vec::new();
    let mut seq: i64 = 0;

    if text.is_empty() {
        return chunks;
    }

    let mut start = 0usize;
    while start < text.len() {
        // FR-001: align byte boundary to a valid UTF-8 char boundary before
        // slicing, so CJK / multi-byte characters are never split mid-codepoint.
        let end_byte = (start + max_chars).min(text.len());
        let end = floor_char_boundary(text, end_byte);

        // Try to break at a paragraph boundary first
        let actual_end = if end < text.len() {
            text[start..end].rfind("\n\n")
                .map(|p| start + p + 2)
                .or_else(|| text[start..end].rfind('\n').map(|p| start + p + 1))
                .or_else(|| text[start..end].rfind(' ').map(|p| start + p + 1))
                // FR-001: rfind fallback must also be char-boundary safe.
                .map(|p| floor_char_boundary(text, p))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProcessingConfig;
    use crate::parse::{BlockType, OkfBlock};

    // T002: 200+ Chinese chars; default chunk boundary (4×chunk_max_tokens bytes)
    // lands inside a Han codepoint without the floor_char_boundary fix.
    const CJK_FIXTURE: &str = "这是一段用于测试分块逻辑的中文文本。\
        它包含超过两百个汉字，以确保默认的字节切块边界会落在某个汉字的中间，\
        从而触发修复前的 panic。每个汉字占用三个字节，因此字节步进会在字符中间断开。\
        修复后使用 floor_char_boundary 对齐到合法的 UTF-8 字符边界，\
        确保所有切块操作都是安全的，不会发生 panic。这段文字还包含表情：😀。\
        继续添加更多内容以超过两百个字符的长度限制，让测试充分覆盖边界情况。";

    // T003: block list with non-contiguous IDs [0, 2, 5].
    fn non_contiguous_blocks() -> Vec<OkfBlock> {
        vec![
            OkfBlock { block_id: 0, block_type: BlockType::Heading, text: "Section A".into(), page: None, bbox: None, from_image: false },
            OkfBlock { block_id: 2, block_type: BlockType::Para, text: "Gap block (id=2)".into(), page: None, bbox: None, from_image: false },
            OkfBlock { block_id: 5, block_type: BlockType::Para, text: "Sparse block (id=5)".into(), page: None, bbox: None, from_image: false },
        ]
    }

    // T009 — FR-001: CJK text must not panic during chunking.
    #[test]
    fn test_chunk_cjk() {
        // Small chunk size forces the byte boundary to land mid-codepoint.
        let cfg = ProcessingConfig { chunk_max_tokens: 20, ..ProcessingConfig::default() };
        // Must not panic:
        let chunks = chunk_text(CJK_FIXTURE, &cfg);
        assert!(!chunks.is_empty(), "expected at least one chunk from CJK fixture");
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(!chunk.text.is_empty(), "chunk {i} text should not be empty");
            // Valid byte indices — would panic here if FR-001 fix is missing.
            let start = chunk.char_start as usize;
            let end = (chunk.char_end as usize).min(CJK_FIXTURE.len());
            let _ = &CJK_FIXTURE[start..end];
        }
    }

    // T010 — FR-002: non-contiguous block_id list must not panic.
    #[test]
    fn test_empty_paragraph_blocks() {
        let blocks = non_contiguous_blocks();
        // build_linear_text must not panic on non-contiguous IDs.
        let (linear_text, spans) = build_linear_text(&blocks);

        // HashMap must contain exactly the three defined block_ids.
        assert!(spans.contains_key(&0));
        assert!(spans.contains_key(&2));
        assert!(spans.contains_key(&5));
        assert_eq!(spans.len(), 3);

        // Verify offsets for block 0 ("Section A" = 9 bytes).
        assert_eq!(spans[&0], (0i64, 9i64));
        // Block 2 starts after "\n\n": offset 11.
        assert_eq!(spans[&2].0, 11i64);

        // Simulate block-insert loop: HashMap::get must handle missing IDs without panic.
        for block_id in [0u32, 1, 2, 3, 4, 5] {
            match spans.get(&block_id) {
                Some(&(lin_start, lin_end)) => {
                    assert!(lin_end <= linear_text.len() as i64);
                    assert!(lin_start >= 0);
                }
                None => { /* expected for 1, 3, 4 — no panic */ }
            }
        }
    }
}
