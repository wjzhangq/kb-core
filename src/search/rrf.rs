use std::collections::HashMap;

use super::bm25::BM25ChunkHit;
use super::vector::VecChunkHit;

#[derive(Debug, Clone)]
pub struct FusedChunk {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub score: f64,
    pub matched_by: Vec<String>,
    pub text: String,
    pub char_start: i64,
    pub char_end: i64,
    pub truncated: bool,
}

/// Reciprocal Rank Fusion: score = Σ 1/(rrfK + rank_i)
pub fn fuse(
    bm25_hits: Vec<BM25ChunkHit>,
    vec_hits: Vec<VecChunkHit>,
    rrf_k: f64,
    top_k: usize,
) -> Vec<FusedChunk> {
    let mut scores: HashMap<i64, FusedChunk> = HashMap::new();

    for (rank, hit) in bm25_hits.iter().enumerate() {
        let rrf_score = 1.0 / (rrf_k + rank as f64 + 1.0);
        let entry = scores.entry(hit.chunk_id).or_insert_with(|| FusedChunk {
            chunk_id: hit.chunk_id,
            doc_id: hit.doc_id,
            score: 0.0,
            matched_by: vec![],
            text: hit.text.clone().unwrap_or_default(),
            char_start: hit.char_start,
            char_end: hit.char_end,
            truncated: hit.truncated,
        });
        entry.score += rrf_score;
        if !entry.matched_by.contains(&"bm25".to_string()) {
            entry.matched_by.push("bm25".to_string());
        }
    }

    for (rank, hit) in vec_hits.iter().enumerate() {
        let rrf_score = 1.0 / (rrf_k + rank as f64 + 1.0);
        let entry = scores.entry(hit.chunk_id).or_insert_with(|| FusedChunk {
            chunk_id: hit.chunk_id,
            doc_id: hit.doc_id,
            score: 0.0,
            matched_by: vec![],
            text: hit.text.clone().unwrap_or_default(),
            char_start: hit.char_start,
            char_end: hit.char_end,
            truncated: hit.truncated,
        });
        entry.score += rrf_score;
        if !entry.matched_by.contains(&"vector".to_string()) {
            entry.matched_by.push("vector".to_string());
        }
    }

    let mut fused: Vec<FusedChunk> = scores.into_values().collect();
    fused.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    fused.truncate(top_k);
    fused
}
