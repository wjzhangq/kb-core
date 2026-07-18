use std::sync::Arc;
use anyhow::Result;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::path::Path;

use crate::config::{EmbeddingModelSpec, ProcessingConfig};

pub mod e5;

pub struct EmbedEngine {
    inner: Arc<e5::E5Engine>,
}

impl EmbedEngine {
    pub fn new(
        model: &EmbeddingModelSpec,
        models_dir: &Path,
        max_cpu_threads: usize,
    ) -> Result<Self> {
        let engine = e5::E5Engine::new(model, models_dir, max_cpu_threads)?;
        Ok(EmbedEngine { inner: Arc::new(engine) })
    }

    /// Embed a batch of passage texts (adds "passage:" prefix internally).
    pub fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.inner.embed_passages(texts)
    }

    /// Embed a single query text (adds "query:" prefix internally).
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed_query(text)
    }
}
