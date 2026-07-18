use std::sync::Arc;
use anyhow::Result;
use tantivy::TantivyDocument;

use super::TantivyIndex;

const COMMIT_BATCH_DOCS: usize = 200;
const COMMIT_INTERVAL_MS: u64 = 1_000;

/// Write a chunk into the tantivy index.
pub fn add_chunk(
    idx: &TantivyIndex,
    chunk_id: u64,
    doc_id: u64,
    doc_type: &str,
    text: &str,
    title: &str,
    path: &str,
) -> Result<()> {
    let s = &idx.schema;
    let mut doc = TantivyDocument::default();
    doc.add_u64(s.chunk_id, chunk_id);
    doc.add_u64(s.doc_id, doc_id);
    doc.add_text(s.doc_type, doc_type);
    doc.add_text(s.text, text);
    doc.add_text(s.title, title);
    doc.add_text(s.path, path);

    idx.with_writer(|iw| {
        iw.add_document(doc)?;
        Ok(())
    })
}

/// Delete all tantivy chunks belonging to a document.
pub fn delete_doc_chunks(idx: &TantivyIndex, doc_id: u64) -> Result<()> {
    let s = &idx.schema;
    idx.with_writer(|iw| {
        let term = tantivy::Term::from_field_u64(s.doc_id, doc_id);
        iw.delete_term(term);
        Ok(())
    })
}

/// Commit pending writes. Idempotent.
pub fn commit(idx: &TantivyIndex) -> Result<()> {
    idx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn open_test_index() -> (Arc<TantivyIndex>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let idx = TantivyIndex::open_or_create(tmp.path(), 1).unwrap();
        (idx, tmp)
    }

    #[test]
    fn reindex_within_single_commit() {
        let (idx, _tmp) = open_test_index();

        add_chunk(&idx, 1, 10, "text", "hello world", "doc", "/a.md").unwrap();
        // Re-index: delete old, add new
        delete_doc_chunks(&idx, 10).unwrap();
        add_chunk(&idx, 1, 10, "text", "hello world updated", "doc", "/a.md").unwrap();
        commit(&idx).unwrap();

        let reader = idx.reader().unwrap();
        let searcher = reader.searcher();
        // Total docs should be 1 after delete+add in same commit
        assert_eq!(searcher.num_docs(), 1);
    }

    #[test]
    fn delete_document_removes_all_its_chunks() {
        let (idx, _tmp) = open_test_index();

        for i in 0u64..3 {
            add_chunk(&idx, i, 42, "text", &format!("chunk {i}"), "doc", "/b.md").unwrap();
        }
        commit(&idx).unwrap();

        delete_doc_chunks(&idx, 42).unwrap();
        commit(&idx).unwrap();

        let reader = idx.reader().unwrap();
        let searcher = reader.searcher();
        assert_eq!(searcher.num_docs(), 0);
    }
}
