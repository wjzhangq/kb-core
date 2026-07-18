use std::path::Path;
use std::sync::Arc;
use anyhow::Result;
use parking_lot::Mutex;
use tantivy::{
    Index, IndexSettings, IndexWriter, ReloadPolicy,
    schema::{TextFieldIndexing, TextOptions, FAST, STORED},
    tokenizer::{SimpleTokenizer, TextAnalyzer},
};
use tantivy_jieba::JiebaTokenizer;

pub mod schema;
pub mod writer;

pub use schema::TantivySchema;

pub struct TantivyIndex {
    pub index: Index,
    pub schema: TantivySchema,
    writer: Mutex<Option<IndexWriter>>,
}

impl TantivyIndex {
    pub fn open_or_create(data_dir: &Path, max_cpu_threads: usize) -> Result<Arc<Self>> {
        let tantivy_dir = data_dir.join("tantivy");
        std::fs::create_dir_all(&tantivy_dir)?;

        let ts = TantivySchema::build();
        let dir = tantivy::directory::MmapDirectory::open(&tantivy_dir)?;

        let index = match Index::open(dir.clone()) {
            Ok(idx) => idx,
            Err(_) => Index::create(dir, ts.schema.clone(), IndexSettings::default())?,
        };

        register_tokenizers(&index);

        let budget = 64 * 1024 * 1024; // 64 MB
        let iw = index.writer_with_num_threads(max_cpu_threads.max(1), budget)?;

        Ok(Arc::new(TantivyIndex {
            index,
            schema: ts,
            writer: Mutex::new(Some(iw)),
        }))
    }

    pub fn with_writer<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut IndexWriter) -> Result<R>,
    {
        let mut guard = self.writer.lock();
        let iw = guard.as_mut().ok_or_else(|| anyhow::anyhow!("index writer already closed"))?;
        f(iw)
    }

    pub fn commit(&self) -> Result<()> {
        self.with_writer(|iw| { iw.commit()?; Ok(()) })
    }

    pub fn close(&self) -> Result<()> {
        let mut guard = self.writer.lock();
        if let Some(mut iw) = guard.take() {
            iw.commit()?;
        }
        Ok(())
    }

    pub fn reader(&self) -> Result<tantivy::IndexReader> {
        let reader = self.index.reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(reader)
    }
}

fn register_tokenizers(index: &Index) {
    let jieba_analyzer = TextAnalyzer::builder(JiebaTokenizer::new())
        .filter(tantivy::tokenizer::LowerCaser)
        .build();
    index.tokenizers().register("jieba_lower", jieba_analyzer);

    index.tokenizers().register(
        "en_lower",
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(tantivy::tokenizer::LowerCaser)
            .build(),
    );
}
