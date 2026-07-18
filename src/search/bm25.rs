use std::sync::Arc;
use anyhow::Result;
use tantivy::{collector::TopDocs, query::QueryParser, ReloadPolicy};
use rusqlite::params;

use crate::db::DbConn;
use crate::tantivy_idx::TantivyIndex;
use super::{RankedChunk, SearchOptions};

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
    let reader = tantivy.reader()?;
    reader.reload()?;
    let searcher = reader.searcher();

    let schema = &tantivy.schema;
    let query_parser = QueryParser::for_index(&tantivy.index, vec![schema.text, schema.title]);
    let query = query_parser.parse_query(query_str)
        .unwrap_or_else(|_| {
            // Fall back to empty query on parse error
            Box::new(tantivy::query::AllQuery)
        });

    let top_docs = searcher.search(&query, &TopDocs::with_limit(opts.top_k))?;

    let mut hits = Vec::with_capacity(top_docs.len());
    for (score, addr) in top_docs {
        let doc = searcher.doc::<tantivy::TantivyDocument>(addr)?;
        let chunk_id = doc.get_first(schema.chunk_id)
            .and_then(|v| v.as_u64()).unwrap_or(0) as i64;
        let doc_id = doc.get_first(schema.doc_id)
            .and_then(|v| v.as_u64()).unwrap_or(0) as i64;

        hits.push(BM25ChunkHit { chunk_id, doc_id, score: score as f64 });
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
