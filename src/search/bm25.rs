use std::sync::Arc;
use anyhow::Result;
use tantivy::{collector::TopDocs, query::QueryParser};
use tantivy::schema::Value; // needed for OwnedValue::as_u64()
use rusqlite::params;

use crate::db::DbConn;
use crate::tantivy_idx::TantivyIndex;
use super::SearchOptions;

#[derive(Debug, Clone)]
pub struct Bm25Hit {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub score: f32,
    pub rank: usize,
}

pub async fn search_bm25(
    tantivy: &Arc<TantivyIndex>,
    query_str: &str,
    opts: &SearchOptions,
    db: &Arc<tokio::sync::Mutex<DbConn>>,
) -> Result<Vec<BM25ChunkHit>> {
    // FR-005: reuse the singleton IndexReader instead of creating a new one each call.
    tantivy.reader().reload()?;
    let searcher = tantivy.reader().searcher();

    let schema = &tantivy.schema;
    let query_parser = QueryParser::for_index(&tantivy.index, vec![schema.text, schema.title]);

    // FR-004: a format-invalid query returns empty results, not a full-corpus scan.
    let query = match query_parser.parse_query(query_str) {
        Ok(q) => q,
        Err(_) => return Ok(vec![]),
    };

    let top_docs = searcher.search(&query, &TopDocs::with_limit(opts.top_k))?;

    let mut hits = Vec::with_capacity(top_docs.len());
    for (score, addr) in top_docs {
        let doc = searcher.doc::<tantivy::TantivyDocument>(addr)?;
        let chunk_id = doc.get_first(schema.chunk_id)
            .and_then(|v| v.as_u64()).unwrap_or(0) as i64;
        let doc_id = doc.get_first(schema.doc_id)
            .and_then(|v| v.as_u64()).unwrap_or(0) as i64;

        // Store minimal hit; fields enriched from SQLite below.
        hits.push(BM25ChunkHit {
            chunk_id,
            doc_id,
            score: score as f64,
            text: None,
            char_start: 0,
            char_end: 0,
            truncated: false,
        });
    }

    // Enrich with chunk data from SQLite
    if hits.is_empty() { return Ok(vec![]); }

    let guard = db.lock().await;
    let mut result = Vec::with_capacity(hits.len());
    for hit in hits {
        if let Ok((text, char_start, char_end, truncated)) = guard.conn.query_row(
            "SELECT text, char_start, char_end, truncated FROM chunks WHERE chunk_id=?1",
            params![hit.chunk_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            )),
        ) {
            result.push(BM25ChunkHit {
                chunk_id: hit.chunk_id,
                doc_id: hit.doc_id,
                score: hit.score,
                text: Some(text),
                char_start,
                char_end,
                truncated: truncated != 0,
            });
        }
    }

    Ok(result)
}

#[derive(Debug, Clone)]
pub struct BM25ChunkHit {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub score: f64,
    pub text: Option<String>,
    pub char_start: i64,
    pub char_end: i64,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Mutex;
    use tempfile::TempDir;
    use crate::tantivy_idx::{TantivyIndex, writer as tw};
    use crate::db::DbConn;
    use crate::search::SearchOptions;

    // T013 — FR-004: a format-invalid query returns empty results, not a full scan.
    #[tokio::test]
    async fn test_bm25_invalid_query() {
        let dir = TempDir::new().unwrap();
        let tantivy = TantivyIndex::open_or_create(dir.path(), 1).unwrap();

        // Non-empty corpus.
        tw::add_chunk(&tantivy, 1, 1, "text", "hello world", "doc1", "/tmp/doc1.txt").unwrap();
        tw::add_chunk(&tantivy, 2, 2, "text", "foo bar baz", "doc2", "/tmp/doc2.txt").unwrap();
        tw::commit(&tantivy).unwrap();

        let db_dir = TempDir::new().unwrap();
        let db = Arc::new(Mutex::new(DbConn::open_writer(db_dir.path()).unwrap()));
        let opts = SearchOptions { top_k: 50, ..Default::default() };

        // "[unclosed" is a malformed tantivy query → must return empty, not all docs.
        let result = search_bm25(&tantivy, "[unclosed bracket", &opts, &db).await.unwrap();
        assert_eq!(result.len(), 0, "invalid query should return 0 results, not full corpus");
    }

    // T024 — FR-005: 1000 searches through the singleton reader must not leak fds.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn test_reader_no_fd_leak() {
        let dir = TempDir::new().unwrap();
        let tantivy = TantivyIndex::open_or_create(dir.path(), 1).unwrap();
        tw::add_chunk(&tantivy, 1, 1, "text", "hello world", "doc", "/tmp/test.txt").unwrap();
        tw::commit(&tantivy).unwrap();

        let db_dir = TempDir::new().unwrap();
        let db = Arc::new(Mutex::new(DbConn::open_writer(db_dir.path()).unwrap()));
        let opts = SearchOptions { top_k: 10, ..Default::default() };

        for _ in 0..10 {
            let _ = search_bm25(&tantivy, "hello", &opts, &db).await.unwrap();
        }
        let fd_before = count_open_fds();
        for _ in 0..1000 {
            let _ = search_bm25(&tantivy, "hello", &opts, &db).await.unwrap();
        }
        let fd_after = count_open_fds();
        let delta = fd_after.saturating_sub(fd_before);
        assert!(delta <= 10, "fd delta after 1000 searches should be ≤ 10, got {}", delta);
    }

    #[cfg(not(target_os = "windows"))]
    fn count_open_fds() -> usize {
        #[cfg(target_os = "linux")]
        { std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0) }
        #[cfg(not(target_os = "linux"))]
        {
            let pid = std::process::id();
            match std::process::Command::new("lsof").args(["-p", &pid.to_string()]).output() {
                Ok(out) => String::from_utf8_lossy(&out.stdout).lines().count().saturating_sub(1),
                Err(_) => 0,
            }
        }
    }
}
