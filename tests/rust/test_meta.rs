// Rust integration tests: chunk→block meta lookup

#[cfg(test)]
mod tests {
    // chunk_to_block_offset_lookup test is in src/search/meta.rs directly.
    // This file adds further integration scenarios.

    #[test]
    fn bbox_aggregation_multi_page() {
        use rusqlite::Connection;
        use kb_core::search::meta::lookup_chunk_meta;

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE blocks (
                doc_id INTEGER, block_id INTEGER, type TEXT, page INTEGER,
                bbox TEXT, from_image INTEGER, lin_start INTEGER, lin_end INTEGER,
                PRIMARY KEY (doc_id, block_id)
             );
             CREATE INDEX idx_blocks_span ON blocks(doc_id, lin_start, lin_end);
             INSERT INTO blocks VALUES (1,0,'heading',1,'[10,20,100,30]',0,0,50);
             INSERT INTO blocks VALUES (1,1,'image_ocr',2,'[0,0,200,150]',1,52,200);",
        ).unwrap();

        let meta = lookup_chunk_meta(&conn, 1, 0, 200).unwrap();
        // Page range should span pages 1–2
        assert_eq!(meta.page_range, Some((1, 2)));
        // from_image should be true (block 1 is image_ocr)
        assert!(meta.from_image);
        // Both block types
        assert!(meta.block_types.contains(&"heading".to_string()));
        assert!(meta.block_types.contains(&"image_ocr".to_string()));
        // Both bboxes present
        assert_eq!(meta.bbox.as_ref().map(|v| v.len()), Some(2));
    }
}
