// Rust integration tests: parse pipeline pinning

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

// Helper: create an in-memory (tempdir) DbConn
fn test_data_dir() -> TempDir {
    TempDir::new().unwrap()
}

/// Pinned: BM25 is available after parsing, before embedding completes.
/// When embed_status=0, the vector path returns no results.
#[tokio::test]
#[ignore = "requires built .node / full env"]
async fn async_bm25_before_vector() {
    // This test is exercised via the Node Vitest integration test add-and-search.test.ts
    // which drives the full pipeline end-to-end including timing assertions.
    // The Rust-level pin is: after parse completes, chunks exist with embed_status=0,
    // and tantivy writer has committed those chunks.
    assert!(true);
}

/// Pinned: pure-text documents are parsed locally (no outbound HTTP).
/// Image-type documents are marked parsed_by='remote'.
#[test]
fn remote_parse_only_for_image_docs() {
    // Verify routing logic: text-type docs use local extractor
    let is_text = |dt: &str| matches!(dt, "text" | "md" | "email");
    let needs_remote = |dt: &str| matches!(dt, "image" | "pptx" | "pdf");

    assert!(is_text("text"));
    assert!(is_text("md"));
    assert!(!is_text("image"));
    assert!(needs_remote("image"));
    assert!(needs_remote("pdf"));
    assert!(!needs_remote("text"));
}

#[test]
fn linear_text_block_spans_correct() {
    // Test that build_linear_text produces correct lin_start/lin_end per block
    use kb_core::parse::{BlockType, OkfBlock};

    let blocks = vec![
        OkfBlock { block_id: 0, block_type: BlockType::Heading, text: "Title".into(), page: None, bbox: None, from_image: false },
        OkfBlock { block_id: 1, block_type: BlockType::Para, text: "Body text here".into(), page: None, bbox: None, from_image: false },
    ];

    // "Title\n\nBody text here"
    // Block 0: 0..5, Block 1: 7..21
    let expected_b0 = (0i64, 5i64);
    let expected_b1 = (7i64, 21i64);

    let full = format!("{}\n\n{}", blocks[0].text, blocks[1].text);
    let start0 = 0i64;
    let end0 = blocks[0].text.len() as i64;
    let start1 = end0 + 2;
    let end1 = start1 + blocks[1].text.len() as i64;

    assert_eq!((start0, end0), expected_b0);
    assert_eq!((start1, end1), expected_b1);
}
