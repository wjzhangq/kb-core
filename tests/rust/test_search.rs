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

    let fused = kb_core::search::rrf::fuse(bm25, vec, 60.0, 10);

    let chunk3 = fused.iter().find(|c| c.chunk_id == 3).expect("chunk 3 in results");
    assert!(chunk3.matched_by.contains(&"bm25".to_string()), "chunk3 should be matched by bm25");
    assert!(chunk3.matched_by.contains(&"vector".to_string()), "chunk3 should be matched by vector");

    let chunk1 = fused.iter().find(|c| c.chunk_id == 1).expect("chunk 1 in results");
    assert_eq!(chunk1.matched_by, vec!["bm25"]);

    let chunk2 = fused.iter().find(|c| c.chunk_id == 2).expect("chunk 2 in results");
    assert_eq!(chunk2.matched_by, vec!["vector"]);
}

// ── T013: BM25 invalid query returns empty (FR-004) ──────────────────────────
/// A format-invalid query string must return Ok(vec![]) instead of AllQuery
/// (full-corpus scan).  Verified by checking result count is 0 on a non-empty index.
#[tokio::test]
async fn test_bm25_invalid_query() {
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tempfile::TempDir;
    use kb_core::tantivy_idx::{TantivyIndex, writer as tw};
    use kb_core::search::{SearchOptions, bm25::search_bm25};
    use kb_core::db::DbConn;

    let dir = TempDir::new().unwrap();
    let tantivy = TantivyIndex::open_or_create(dir.path(), 1).unwrap();

    // Index a few documents so the corpus is non-empty.
    tw::add_chunk(&tantivy, 1, 1, "text", "hello world", "doc1", "/tmp/doc1.txt").unwrap();
    tw::add_chunk(&tantivy, 2, 2, "text", "foo bar baz", "doc2", "/tmp/doc2.txt").unwrap();
    tw::commit(&tantivy).unwrap();

    let db_dir = TempDir::new().unwrap();
    let db = Arc::new(Mutex::new(DbConn::open_writer(db_dir.path()).unwrap()));

    let opts = SearchOptions { top_k: 50, ..Default::default() };

    // "[unclosed" is a malformed tantivy query — must return empty, not all docs.
    let result = search_bm25(&tantivy, "[unclosed bracket", &opts, &db).await.unwrap();
    assert_eq!(result.len(), 0, "invalid query should return 0 results, not full corpus");
}

// ── T024: IndexReader singleton does not leak file descriptors (FR-005) ───────
/// Run 1000 BM25 searches and assert the fd count before vs after stays stable.
/// Skipped on Windows (use quickstart.md Scenario 5 there).
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn test_reader_no_fd_leak() {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tempfile::TempDir;
    use kb_core::tantivy_idx::{TantivyIndex, writer as tw};
    use kb_core::search::{SearchOptions, bm25::search_bm25};
    use kb_core::db::DbConn;

    let dir = TempDir::new().unwrap();
    let tantivy = TantivyIndex::open_or_create(dir.path(), 1).unwrap();
    tw::add_chunk(&tantivy, 1, 1, "text", "hello world", "doc", "/tmp/test.txt").unwrap();
    tw::commit(&tantivy).unwrap();

    let db_dir = TempDir::new().unwrap();
    let db = Arc::new(Mutex::new(DbConn::open_writer(db_dir.path()).unwrap()));
    let opts = SearchOptions { top_k: 10, ..Default::default() };

    // Warm-up — don't count setup allocations
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

/// Count open file descriptors for the current process.
#[cfg(not(target_os = "windows"))]
fn count_open_fds() -> usize {
    // macOS: lsof -p <pid> | wc -l  (fast enough for tests)
    // Linux: /proc/self/fd
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/fd").map(|d| d.count()).unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let pid = std::process::id();
        let out = std::process::Command::new("lsof")
            .args(["-p", &pid.to_string()])
            .output()
            .unwrap_or_else(|_| std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: vec![],
                stderr: vec![],
            });
        String::from_utf8_lossy(&out.stdout).lines().count().saturating_sub(1)
    }
}

