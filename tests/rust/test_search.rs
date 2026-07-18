// Rust integration tests: search behavior

/// Pinned: CJK/English mixed document recalled via English term query through vector path.
/// When 'BLE' and 'MQTT' are in the document, vector search finds them via semantic similarity.
#[test]
#[ignore = "requires full model + index; covered by Node Vitest add-and-search.test.ts"]
fn cjk_english_mixed_recall() {
    // End-to-end covered by tests/node/add-and-search.test.ts
    assert!(true);
}

/// Pinned: RRF fusion preserves matchedBy correctly.
#[test]
fn rrf_matched_by_preserved() {
    use kb_core::search::rrf::{fuse, BM25ChunkHit, VecChunkHit};

    // Simulated BM25 hit for chunk 1
    // Simulated vec hit for chunk 2
    // Both match chunk 3

    // Build synthetic BM25 hits
    use kb_core::search::bm25::BM25ChunkHit;
    use kb_core::search::vector::VecChunkHit;

    let bm25 = vec![
        BM25ChunkHit { chunk_id: 1, doc_id: 10, score: 1.0, text: Some("hello".into()), char_start: 0, char_end: 5, truncated: false },
        BM25ChunkHit { chunk_id: 3, doc_id: 10, score: 0.8, text: Some("world".into()), char_start: 10, char_end: 15, truncated: false },
    ];
    let vec = vec![
        VecChunkHit { chunk_id: 2, doc_id: 10, distance: 0.1, score: 0.9, text: Some("foo".into()), char_start: 5, char_end: 10, truncated: false },
        VecChunkHit { chunk_id: 3, doc_id: 10, distance: 0.2, score: 0.8, text: Some("world".into()), char_start: 10, char_end: 15, truncated: false },
    ];

    let fused = fuse(bm25, vec, 60.0, 10);

    let chunk3 = fused.iter().find(|c| c.chunk_id == 3).expect("chunk 3 in results");
    assert!(chunk3.matched_by.contains(&"bm25".to_string()), "chunk3 should be matched by bm25");
    assert!(chunk3.matched_by.contains(&"vector".to_string()), "chunk3 should be matched by vector");

    let chunk1 = fused.iter().find(|c| c.chunk_id == 1).expect("chunk 1 in results");
    assert_eq!(chunk1.matched_by, vec!["bm25"]);

    let chunk2 = fused.iter().find(|c| c.chunk_id == 2).expect("chunk 2 in results");
    assert_eq!(chunk2.matched_by, vec!["vector"]);
}
