pub mod bm25;
pub mod meta;
pub mod rrf;
pub mod vector;

use std::sync::Arc;
use std::time::Instant;
use anyhow::Result;

use crate::config::{InferenceConfig, KBConfig};
use crate::db::DbConn;
use crate::embed::EmbedEngine;
use crate::tantivy_idx::TantivyIndex;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub top_k: usize,
    pub top_n: usize,
    pub rrf_k: f64,
    pub aggregate: AggregateMode,
    pub filter_doc_types: Vec<String>,
    pub filter_paths: Vec<String>,
    pub syntax: QuerySyntax,
    pub max_chars_per_chunk: usize,
    pub include_text: bool,
    pub require_vector: bool,
}

#[derive(Debug, Clone)]
pub enum AggregateMode { Max, Sum, Top2Sum }

#[derive(Debug, Clone)]
pub enum QuerySyntax { Text, Fielded, Raw }

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            top_k: 50,
            top_n: 5,
            rrf_k: 60.0,
            aggregate: AggregateMode::Max,
            filter_doc_types: vec![],
            filter_paths: vec![],
            syntax: QuerySyntax::Text,
            max_chars_per_chunk: 800,
            include_text: true,
            require_vector: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchTiming {
    pub parse_ms: f64,
    pub bm25_ms: f64,
    pub embed_ms: f64,
    pub vec_ms: f64,
    pub rrf_ms: f64,
    pub aggregate_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone)]
pub struct RankedChunk {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub text: String,
    pub char_start: i64,
    pub char_end: i64,
    pub truncated: bool,
    pub score: f64,
    pub matched_by: Vec<String>,
}

#[derive(Debug)]
pub struct SearchResultDoc {
    pub doc_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub score: f64,
    pub chunks: Vec<RankedChunkWithMeta>,
}

#[derive(Debug)]
pub struct RankedChunkWithMeta {
    pub chunk_id: i64,
    pub text: String,
    pub truncated: bool,
    pub char_offset: (i64, i64),
    pub page_range: Option<(u32, u32)>,
    pub bbox: Option<Vec<(u32, [f32; 4])>>,
    pub block_types: Vec<String>,
    pub from_image: bool,
    pub matched_by: Vec<String>,
    pub score: f64,
}

pub async fn run_search(
    query: &str,
    opts: &SearchOptions,
    config: &KBConfig,
    db: &Arc<tokio::sync::Mutex<DbConn>>,
    tantivy: &Arc<TantivyIndex>,
    embed_engine: &Option<Arc<EmbedEngine>>,
) -> Result<(Vec<SearchResultDoc>, SearchTiming, String, f64, Option<String>)> {
    let total_start = Instant::now();

    // 1. Parse query + query embedding
    let parse_start = Instant::now();
    let sanitized = match opts.syntax {
        QuerySyntax::Text => escape_query(query),
        _ => query.to_string(),
    };
    let query_embedding: Option<Vec<f32>> = if let Some(engine) = embed_engine {
        Some(engine.embed_query(&sanitized)?)
    } else {
        None
    };
    let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
    let embed_ms = 0.0; // included in parse for now

    // 2. BM25 search
    let bm25_start = Instant::now();
    let bm25_hits = bm25::search_bm25(tantivy, &sanitized, opts, db).await?;
    let bm25_ms = bm25_start.elapsed().as_secs_f64() * 1000.0;

    // 3. Vector search
    let vec_start = Instant::now();
    let (vec_hits, vec_coverage) = if let Some(ref emb) = query_embedding {
        if !opts.require_vector || vec_coverage_ok(db).await? {
            vector::search_vector(db, emb, opts).await?
        } else {
            (vec![], 0.0)
        }
    } else {
        (vec![], 0.0)
    };
    let vec_ms = vec_start.elapsed().as_secs_f64() * 1000.0;
    let had_vec_hits = !vec_hits.is_empty();

    // 4. RRF fusion
    let rrf_start = Instant::now();
    let fused = rrf::fuse(bm25_hits, vec_hits, opts.rrf_k, opts.top_k);
    let rrf_ms = rrf_start.elapsed().as_secs_f64() * 1000.0;

    // 5. Meta lookup + aggregation
    let agg_start = Instant::now();
    let results = aggregate(fused, opts, db).await?;
    let aggregate_ms = agg_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

    let mode = if query_embedding.is_some() && had_vec_hits {
        "bm25+vec".to_string()
    } else {
        "bm25-only".to_string()
    };

    let degraded = build_degraded_reason(config, embed_engine, vec_coverage, opts);

    let timing = SearchTiming {
        parse_ms, bm25_ms, embed_ms, vec_ms, rrf_ms, aggregate_ms, total_ms,
    };

    Ok((results, timing, mode, vec_coverage, degraded))
}

async fn vec_coverage_ok(db: &Arc<tokio::sync::Mutex<DbConn>>) -> Result<bool> {
    let guard = db.lock().await;
    let (done, total): (i64, i64) = guard.conn.query_row(
        "SELECT SUM(CASE WHEN embed_status=1 THEN 1 ELSE 0 END), COUNT(*) FROM chunks",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if total == 0 { return Ok(false); }
    Ok(done as f64 / total as f64 >= 0.01)
}

async fn aggregate(
    fused: Vec<rrf::FusedChunk>,
    opts: &SearchOptions,
    db: &Arc<tokio::sync::Mutex<DbConn>>,
) -> Result<Vec<SearchResultDoc>> {
    if fused.is_empty() { return Ok(vec![]); }

    let guard = db.lock().await;

    // Group chunks by doc_id
    let mut by_doc: std::collections::HashMap<i64, Vec<rrf::FusedChunk>> = std::collections::HashMap::new();
    for chunk in fused {
        by_doc.entry(chunk.doc_id).or_default().push(chunk);
    }

    let mut doc_scores: Vec<(i64, f64, Vec<rrf::FusedChunk>)> = by_doc.into_iter().map(|(doc_id, chunks)| {
        let score = match opts.aggregate {
            AggregateMode::Max => chunks.iter().map(|c| c.score).fold(f64::NEG_INFINITY, f64::max),
            AggregateMode::Sum => chunks.iter().map(|c| c.score).sum(),
            AggregateMode::Top2Sum => {
                let mut scores: Vec<f64> = chunks.iter().map(|c| c.score).collect();
                scores.sort_by(|a, b| b.partial_cmp(a).unwrap());
                scores.iter().take(2).sum()
            }
        };
        (doc_id, score, chunks)
    }).collect();

    doc_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    doc_scores.truncate(opts.top_n);

    let mut results = Vec::with_capacity(doc_scores.len());
    for (doc_id, score, chunks) in doc_scores {
        let (path, title): (String, Option<String>) = guard.conn.query_row(
            "SELECT path, title FROM documents WHERE doc_id=?1",
            rusqlite::params![doc_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let mut ranked_chunks = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let meta = meta::lookup_chunk_meta(&guard.conn, doc_id, chunk.char_start, chunk.char_end)?;
            let text = if opts.include_text {
                let mut t = chunk.text.clone();
                if t.chars().count() > opts.max_chars_per_chunk {
                    t = t.chars().take(opts.max_chars_per_chunk).collect();
                }
                t
            } else {
                String::new()
            };

            ranked_chunks.push(RankedChunkWithMeta {
                chunk_id: chunk.chunk_id,
                text,
                truncated: chunk.truncated,
                char_offset: (chunk.char_start, chunk.char_end),
                page_range: meta.page_range,
                bbox: meta.bbox,
                block_types: meta.block_types,
                from_image: meta.from_image,
                matched_by: chunk.matched_by,
                score: chunk.score,
            });
        }

        results.push(SearchResultDoc { doc_id, path, title, score, chunks: ranked_chunks });
    }

    Ok(results)
}

fn build_degraded_reason(
    config: &KBConfig,
    engine: &Option<Arc<EmbedEngine>>,
    vec_coverage: f64,
    opts: &SearchOptions,
) -> Option<String> {
    if matches!(config.inference, InferenceConfig::Bm25Only) {
        return Some("bm25-only mode configured".into());
    }
    if engine.is_none() {
        return Some("embedding model not loaded".into());
    }
    if vec_coverage < 0.01 {
        return Some("vector index empty — still indexing".into());
    }
    None
}

fn escape_query(q: &str) -> String {
    // Escape tantivy special chars for 'text' mode
    let mut out = String::with_capacity(q.len());
    for c in q.chars() {
        if "+-&|!(){}[]^\"~*?:\\/".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
