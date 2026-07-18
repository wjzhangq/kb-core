use std::sync::Arc;
use anyhow::Result;
use rusqlite::params;

use crate::db::DbConn;
use super::SearchOptions;

#[derive(Debug, Clone)]
pub struct VecChunkHit {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub distance: f64,
    pub score: f64,
    pub text: Option<String>,
    pub char_start: i64,
    pub char_end: i64,
    pub truncated: bool,
}

/// Full brute-force scan using sqlite-vec cosine distance.
pub async fn search_vector(
    db: &Arc<tokio::sync::Mutex<DbConn>>,
    query_embedding: &[f32],
    opts: &SearchOptions,
) -> Result<(Vec<VecChunkHit>, f64)> {
    let guard = db.lock().await;

    let emb_json = serde_json::to_string(query_embedding)?;

    // Compute vectorCoverage
    let (embed_done, chunk_total): (i64, i64) = guard.conn.query_row(
        "SELECT SUM(CASE WHEN embed_status=1 THEN 1 ELSE 0 END), COUNT(*) FROM chunks",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let vec_coverage = if chunk_total > 0 {
        embed_done as f64 / chunk_total as f64
    } else {
        0.0
    };

    // Query chunks_vec using sqlite-vec distance function
    let mut stmt = guard.conn.prepare(
        "SELECT cv.chunk_id, c.doc_id, c.text, c.char_start, c.char_end, c.truncated,
                vec_distance_cosine(cv.embedding, ?1) AS distance
         FROM chunks_vec cv
         JOIN chunks c ON c.chunk_id = cv.chunk_id
         WHERE c.embed_status = 1
         ORDER BY distance ASC
         LIMIT ?2",
    )?;

    let hits: Vec<VecChunkHit> = stmt.query_map(
        params![emb_json, opts.top_k as i64],
        |row| {
            let distance: f64 = row.get(6)?;
            Ok(VecChunkHit {
                chunk_id: row.get(0)?,
                doc_id: row.get(1)?,
                distance,
                score: 1.0 - distance,
                text: row.get(2)?,
                char_start: row.get(3)?,
                char_end: row.get(4)?,
                truncated: row.get::<_, i64>(5)? != 0,
            })
        },
    )?
    .filter_map(|r| r.ok())
    .collect();

    Ok((hits, vec_coverage))
}
