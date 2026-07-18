use anyhow::Result;
use rusqlite::{params, Connection};

#[derive(Debug, Default)]
pub struct ChunkMeta {
    pub page_range: Option<(u32, u32)>,
    pub bbox: Option<Vec<(u32, [f32; 4])>>,
    pub block_types: Vec<String>,
    pub from_image: bool,
}

/// Resolve a chunk's [char_start, char_end) to block metadata via `idx_blocks_span`.
pub fn lookup_chunk_meta(
    conn: &Connection,
    doc_id: i64,
    char_start: i64,
    char_end: i64,
) -> Result<ChunkMeta> {
    let mut stmt = conn.prepare_cached(
        "SELECT type, page, bbox, from_image
         FROM blocks
         WHERE doc_id=?1
           AND lin_start < ?3
           AND lin_end > ?2
         ORDER BY block_id",
    )?;

    struct BlockRow {
        block_type: String,
        page: Option<u32>,
        bbox_json: Option<String>,
        from_image: bool,
    }

    let rows: Vec<BlockRow> = stmt.query_map(
        params![doc_id, char_start, char_end],
        |row| {
            Ok(BlockRow {
                block_type: row.get(0)?,
                page: row.get::<_, Option<i64>>(1)?.map(|p| p as u32),
                bbox_json: row.get(2)?,
                from_image: row.get::<_, i64>(3)? != 0,
            })
        },
    )?
    .filter_map(|r| r.ok())
    .collect();

    if rows.is_empty() {
        return Ok(ChunkMeta::default());
    }

    let mut pages: Vec<u32> = vec![];
    let mut bbox_list: Vec<(u32, [f32; 4])> = vec![];
    let mut block_types: Vec<String> = vec![];
    let mut from_image = false;

    for row in &rows {
        if row.from_image { from_image = true; }
        if let Some(p) = row.page { pages.push(p); }
        if !block_types.contains(&row.block_type) {
            block_types.push(row.block_type.clone());
        }
        if let (Some(p), Some(json)) = (row.page, &row.bbox_json) {
            if let Ok(arr) = serde_json::from_str::<Vec<f32>>(json) {
                if arr.len() == 4 {
                    bbox_list.push((p, [arr[0], arr[1], arr[2], arr[3]]));
                }
            }
        }
    }

    let page_range = if pages.is_empty() {
        None
    } else {
        let min = *pages.iter().min().unwrap();
        let max = *pages.iter().max().unwrap();
        Some((min, max))
    };

    let bbox = if bbox_list.is_empty() { None } else { Some(bbox_list) };

    block_types.sort();
    block_types.dedup();

    Ok(ChunkMeta { page_range, bbox, block_types, from_image })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE blocks (
                doc_id INTEGER, block_id INTEGER, type TEXT, page INTEGER,
                bbox TEXT, from_image INTEGER, lin_start INTEGER, lin_end INTEGER,
                PRIMARY KEY (doc_id, block_id)
            );
            CREATE INDEX idx_blocks_span ON blocks(doc_id, lin_start, lin_end);",
        ).unwrap();
        conn
    }

    #[test]
    fn chunk_to_block_offset_lookup() {
        let conn = setup_db();
        // Block 0: chars 0..50, Block 1: chars 52..120
        conn.execute_batch(
            "INSERT INTO blocks VALUES (1,0,'heading',1,NULL,0,0,50);
             INSERT INTO blocks VALUES (1,1,'para',1,NULL,0,52,120);",
        ).unwrap();

        let meta = lookup_chunk_meta(&conn, 1, 10, 80).unwrap();
        assert_eq!(meta.page_range, Some((1, 1)));
        assert!(meta.block_types.contains(&"heading".to_string()));
        assert!(meta.block_types.contains(&"para".to_string()));
        assert!(!meta.from_image);
    }
}
