#![deny(clippy::all)]
#![allow(clippy::module_inception)]

use napi_derive::napi;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::Mutex;
use anyhow::Result;
use rusqlite::params;

pub mod config;
pub mod db;
pub mod embed;
pub mod lock;
pub mod parse;
pub mod pipeline;
pub mod search;
pub mod tantivy_idx;
pub mod tempfile;
pub mod thread_pool;

use config::{KBConfig, EmbeddingModelSpec, JsKBConfig, JsAddResult, JsSearchOptions, JsSearchResponse,
             JsKBStatus, JsStatusWarning, JsDegraded, JsSearchResult, JsChunkResult, JsBbox,
             JsSearchTiming, JsEmbeddingModelSpec, InferenceConfig};
use db::DbConn;
use embed::EmbedEngine;
use tantivy_idx::TantivyIndex;
use search::{SearchOptions, AggregateMode, QuerySyntax};

// ── Custom error types ────────────────────────────────────────────────────

#[napi]
pub struct KBLockErrorClass {}

#[napi]
pub struct KBModelMismatchErrorClass {}

#[napi]
pub struct ModelNotFoundErrorClass {}

// ── KnowledgeBase ─────────────────────────────────────────────────────────

struct KBInner {
    config: Arc<KBConfig>,
    db: Arc<Mutex<DbConn>>,
    tantivy: Arc<TantivyIndex>,
    parse_tx: tokio::sync::mpsc::Sender<i64>,
    embed_tx: tokio::sync::mpsc::Sender<i64>,
    embed_engine: Option<Arc<EmbedEngine>>,
    closed: AtomicBool,
}

#[napi]
pub struct KnowledgeBase {
    inner: Arc<KBInner>,
    // Owns the Tokio runtime for the lifetime of the KnowledgeBase; never read
    // directly, but dropping it would shut the runtime down.
    #[allow(dead_code)]
    rt: Arc<tokio::runtime::Runtime>,
}

#[napi]
impl KnowledgeBase {
    #[napi(constructor)]
    pub fn new(options: JsKBConfig) -> napi::Result<Self> {
        let config = KBConfig::from_js(options)
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))?;
        let config = Arc::new(config);

        let max_threads = config.system.max_cpu_threads;
        let rt = Arc::new(
            thread_pool::build_tokio_runtime(max_threads)
                .map_err(|e| napi::Error::from_reason(format!("runtime: {:#}", e)))?
        );
        thread_pool::configure_rayon(max_threads);

        let data_dir = PathBuf::from(&config.data_dir);
        let inner = rt.block_on(async {
            KBInner::open(config.clone(), &data_dir).await
        }).map_err(|e| napi::Error::from_reason(format!("{:#}", e)))?;

        Ok(KnowledgeBase { inner: Arc::new(inner), rt })
    }

    #[napi]
    pub async fn add(&self, path: napi::Either<String, Vec<String>>) -> napi::Result<Vec<JsAddResult>> {
        let paths: Vec<String> = match path {
            napi::Either::A(p) => vec![p],
            napi::Either::B(ps) => ps,
        };
        self.inner.add(paths).await
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))
    }

    #[napi]
    pub async fn search(
        &self,
        query: String,
        options: Option<JsSearchOptions>,
    ) -> napi::Result<JsSearchResponse> {
        self.inner.search(query, options).await
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))
    }

    #[napi]
    pub async fn status(&self) -> napi::Result<JsKBStatus> {
        self.inner.status().await
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))
    }

    #[napi]
    pub async fn reindex_embeddings(&self, model: JsEmbeddingModelSpec) -> napi::Result<()> {
        self.inner.reindex_embeddings(model).await
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))
    }

    #[napi]
    pub async fn close(&self) -> napi::Result<()> {
        self.inner.close().await
            .map_err(|e| napi::Error::from_reason(format!("{:#}", e)))
    }
}

// ── KBInner implementation ────────────────────────────────────────────────

impl KBInner {
    async fn open(config: Arc<KBConfig>, data_dir: &Path) -> Result<Self> {
        // Clean leftover tmp files
        crate::tempfile::cleanup_tmp_dir(data_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to clean tmp dir: {}", e);
        });

        let db_conn = DbConn::open_writer(data_dir)?;
        let model_tag = compute_model_tag_for_config(&config);
        validate_model_tag(&db_conn, &model_tag)?;
        store_model_tag(&db_conn, &model_tag)?;

        let db = Arc::new(Mutex::new(db_conn));

        let tantivy = TantivyIndex::open_or_create(data_dir, config.system.max_cpu_threads)?;

        // Initialize embedding engine
        let embed_engine: Option<Arc<EmbedEngine>> = match &config.inference {
            InferenceConfig::LocalFirst { model, models_dir, .. } => {
                let mdir = PathBuf::from(models_dir.as_deref().unwrap_or(
                    // Default: <package_dir>/models/
                    concat!(env!("CARGO_MANIFEST_DIR"), "/models"),
                ));
                match EmbedEngine::new(model, &mdir, config.system.max_cpu_threads) {
                    Ok(e) => Some(Arc::new(e)),
                    Err(e) => {
                        tracing::warn!("embedding engine init failed (bm25-only fallback): {}", e);
                        None
                    }
                }
            }
            _ => None,
        };

        let embed_tx = pipeline::embed::start_embed_queue(
            config.clone(),
            db.clone(),
            embed_engine.clone(),
        );

        let parse_tx = pipeline::parse::start_parse_queue(
            config.clone(),
            db.clone(),
            tantivy.clone(),
            embed_tx.clone(),
            config.processing.parse_concurrency,
        );

        Ok(KBInner {
            config,
            db,
            tantivy,
            parse_tx,
            embed_tx,
            embed_engine,
            closed: AtomicBool::new(false),
        })
    }

    async fn add(&self, paths: Vec<String>) -> Result<Vec<JsAddResult>> {
        let now = now_ms();
        let mut results = Vec::with_capacity(paths.len());
        let mut to_enqueue: Vec<i64> = Vec::new();

        // FR-007: collect all doc_ids inside the lock scope, then drop the guard
        // before awaiting parse_tx.send().  Holding a tokio::sync::Mutex across
        // an .await while the parse worker also needs the same lock → deadlock
        // when the channel is full.
        {
            let guard = self.db.lock().await;
            for path in &paths {
                let doc_type = detect_doc_type(path);
                let title = std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned());

                // INSERT OR IGNORE — idempotent
                let changes = guard.conn.execute(
                    "INSERT OR IGNORE INTO documents(path, title, doc_type, status, added_at, updated_at)
                     VALUES (?1, ?2, ?3, 'pending_parse', ?4, ?5)",
                    params![path, title, doc_type, now, now],
                )?;

                if changes > 0 {
                    let doc_id: i64 = guard.conn.last_insert_rowid();
                    to_enqueue.push(doc_id);
                    results.push(JsAddResult { doc_id, path: path.clone(), status: "pending_parse".into() });
                } else {
                    let doc_id: i64 = guard.conn.query_row(
                        "SELECT doc_id FROM documents WHERE path=?1",
                        params![path],
                        |row| row.get(0),
                    )?;
                    results.push(JsAddResult { doc_id, path: path.clone(), status: "already_indexed".into() });
                }
            }
        } // guard dropped here — lock released before any .await

        // Safe to await channel sends now that the DB lock is free.
        for doc_id in to_enqueue {
            let _ = self.parse_tx.send(doc_id).await;
        }

        Ok(results)
    }

    async fn search(&self, query: String, options: Option<JsSearchOptions>) -> Result<JsSearchResponse> {
        let opts = options.map(js_opts_to_opts).unwrap_or_default();

        let (results, timing, mode, vec_coverage, degraded) = search::run_search(
            &query, &opts, &self.config, &self.db, &self.tantivy, &self.embed_engine,
        ).await?;

        let js_results: Vec<JsSearchResult> = results.into_iter().map(|r| {
            JsSearchResult {
                doc_id: r.doc_id,
                path: r.path,
                title: r.title,
                score: r.score,
                chunks: r.chunks.into_iter().map(|c| JsChunkResult {
                    chunk_id: c.chunk_id,
                    text: c.text,
                    truncated: c.truncated,
                    char_offset: vec![c.char_offset.0, c.char_offset.1],
                    page_range: c.page_range.map(|(a, b)| vec![a, b]),
                    bbox: c.bbox.map(|bvec| bvec.into_iter().map(|(p, r)| JsBbox {
                        page: p, rect: r.iter().map(|&f| f as f64).collect(),
                    }).collect()),
                    block_types: c.block_types,
                    from_image: c.from_image,
                    matched_by: c.matched_by,
                    score: c.score,
                }).collect(),
            }
        }).collect();

        Ok(JsSearchResponse {
            results: js_results,
            timing: JsSearchTiming {
                parse_ms: timing.parse_ms,
                bm25_ms: timing.bm25_ms,
                embed_ms: timing.embed_ms,
                vec_ms: timing.vec_ms,
                rrf_ms: timing.rrf_ms,
                aggregate_ms: timing.aggregate_ms,
                total_ms: timing.total_ms,
            },
            mode,
            vector_coverage: vec_coverage,
            degraded: degraded.map(|r| JsDegraded { reason: r }),
        })
    }

    async fn status(&self) -> Result<JsKBStatus> {
        let guard = self.db.lock().await;

        // Document counts. COALESCE guards against NULL on an empty table.
        let (total, pending_parse, parsing, parsed, indexed, parse_failed): (i64,i64,i64,i64,i64,i64) =
            guard.conn.query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN status='pending_parse' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='parsing'       THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='parsed'        THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='indexed'       THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='parse_failed'  THEN 1 ELSE 0 END), 0)
                 FROM documents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )?;

        // Chunk embed counts. COALESCE guards against NULL on an empty table.
        let (chunk_total, chunk_embed_done, chunk_embed_pending, chunk_embed_failed): (i64,i64,i64,i64) =
            guard.conn.query_row(
                "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN embed_status=1 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN embed_status=0 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN embed_status=2 THEN 1 ELSE 0 END), 0)
                 FROM chunks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

        let vec_coverage = if chunk_total > 0 { chunk_embed_done as f64 / chunk_total as f64 } else { 0.0 };

        // WAL check
        let journal_mode: String = guard.conn.query_row(
            "PRAGMA journal_mode", [], |row| row.get(0)
        ).unwrap_or_else(|_| "unknown".into());
        let wal_enabled = journal_mode.to_lowercase() == "wal";

        // Writer lock held (this instance holds it if DbConn has a lock)
        let writer_lock_held = guard._lock.is_some();

        // Model readiness
        let model_ready = self.embed_engine.is_some();

        // Warnings
        let mut warnings: Vec<JsStatusWarning> = vec![];

        if parse_failed > 0 {
            let failed_ids: Vec<i64> = {
                let mut stmt = guard.conn.prepare(
                    "SELECT doc_id FROM documents WHERE status='parse_failed'"
                )?;
                // Pre-existing fix (E0597): bind query_map result to a local so the
                // MappedRows temporary is dropped before guard, not after the block.
                let mapped = stmt.query_map([], |row| row.get(0))?;
                mapped.filter_map(|r| r.ok()).collect()
            };
            warnings.push(JsStatusWarning {
                r#type: "parse_failed".into(),
                message: format!("{} document(s) failed to parse", parse_failed),
                doc_ids: Some(failed_ids),
            });
        }

        // Check for legacy docs missing blocks (missing_meta warning)
        let missing_meta_count: i64 = guard.conn.query_row(
            "SELECT COUNT(DISTINCT d.doc_id) FROM documents d
             WHERE d.status IN ('parsed','indexed')
               AND NOT EXISTS (SELECT 1 FROM blocks b WHERE b.doc_id=d.doc_id)",
            [], |row| row.get(0),
        ).unwrap_or(0);

        if missing_meta_count > 0 {
            let ids: Vec<i64> = {
                let mut stmt = guard.conn.prepare(
                    "SELECT DISTINCT d.doc_id FROM documents d
                     WHERE d.status IN ('parsed','indexed')
                       AND NOT EXISTS (SELECT 1 FROM blocks b WHERE b.doc_id=d.doc_id)"
                )?;
                let mapped = stmt.query_map([], |row| row.get(0))?;
                mapped.filter_map(|r| r.ok()).collect()
            };
            warnings.push(JsStatusWarning {
                r#type: "missing_meta".into(),
                message: format!("{} document(s) missing block metadata (re-index needed)", missing_meta_count),
                doc_ids: Some(ids),
            });
        }

        if !model_ready {
            warnings.push(JsStatusWarning {
                r#type: "model_not_found".into(),
                message: "Embedding model files not found. Run postinstall or set KB_MODELS_DIR.".into(),
                doc_ids: None,
            });
        }

        if !wal_enabled {
            warnings.push(JsStatusWarning {
                r#type: "wal_disabled".into(),
                message: "SQLite WAL mode is not enabled (unexpected)".into(),
                doc_ids: None,
            });
        }

        Ok(JsKBStatus {
            total, pending_parse, parsing, parsed, indexed, parse_failed,
            vector_coverage: vec_coverage,
            chunk_total, chunk_embed_done, chunk_embed_pending, chunk_embed_failed,
            wal_enabled,
            writer_lock_held,
            model_ready,
            warnings,
        })
    }

    async fn reindex_embeddings(&self, model: JsEmbeddingModelSpec) -> Result<()> {
        let spec = EmbeddingModelSpec {
            name: model.name.clone(),
            dim: model.dim as usize,
            quantization: model.quantization.unwrap_or_else(|| "int8".into()),
        };
        let new_tag = compute_model_tag(&spec);

        let guard = self.db.lock().await;
        // Clear chunks_vec
        guard.conn.execute("DELETE FROM chunks_vec", [])?;
        // Reset embed_status=0 for all chunks
        guard.conn.execute("UPDATE chunks SET embed_status=0", [])?;
        // Reset indexed→parsed for all documents
        guard.conn.execute("UPDATE documents SET status='parsed' WHERE status='indexed'", [])?;
        // Update model_tag
        guard.conn.execute(
            "INSERT OR REPLACE INTO kb_meta(key, value) VALUES ('model_tag', ?1)",
            params![new_tag],
        )?;
        drop(guard);

        // Re-enqueue all parsed docs for embedding
        let doc_ids: Vec<i64> = {
            let guard = self.db.lock().await;
            let mut stmt = guard.conn.prepare("SELECT doc_id FROM documents WHERE status='parsed'")?;
            let mapped = stmt.query_map([], |row| row.get(0))?;
            mapped.filter_map(|r| r.ok()).collect()
        };

        for doc_id in doc_ids {
            let _ = self.embed_tx.send(doc_id).await;
        }

        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(()); // already closed — idempotent
        }

        // Commit tantivy
        self.tantivy.close()?;

        // SQLite closes on Drop (DbConn contains Connection which drops on close)
        // flock releases on Drop of KBLock inside DbConn

        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn detect_doc_type(path: &str) -> &'static str {
    let p = std::path::Path::new(path);
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("pdf") => "pdf",
        Some("docx") => "docx",
        Some("pptx") => "pptx",
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") => "image",
        Some("eml") => "email",
        Some("md") | Some("markdown") | Some("txt") | Some("rst") => "text",
        _ => "text",
    }
}

fn compute_model_tag_for_config(config: &KBConfig) -> String {
    match &config.inference {
        InferenceConfig::LocalFirst { model, .. } => compute_model_tag(model),
        InferenceConfig::Remote { model, .. } => compute_model_tag(model),
        InferenceConfig::Bm25Only => String::new(),
    }
}

fn compute_model_tag(spec: &EmbeddingModelSpec) -> String {
    use sha2::{Sha256, Digest};
    let input = format!("{}|{}|{}", spec.name, spec.dim, spec.quantization);
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(&hash[..8]) // 16 hex chars from 8 bytes
}

fn validate_model_tag(conn: &DbConn, current_tag: &str) -> Result<()> {
    if current_tag.is_empty() { return Ok(()); }

    let stored: Option<String> = conn.conn.query_row(
        "SELECT value FROM kb_meta WHERE key='model_tag'",
        [],
        |row| row.get(0),
    ).optional()?;

    if let Some(stored_tag) = stored {
        if !stored_tag.is_empty() && stored_tag != current_tag {
            return Err(anyhow::anyhow!(
                "KBModelMismatchError: model_tag mismatch. expected={} found={}",
                current_tag, stored_tag
            ));
        }
    }
    Ok(())
}

fn store_model_tag(conn: &DbConn, tag: &str) -> Result<()> {
    if tag.is_empty() { return Ok(()); }
    conn.conn.execute(
        "INSERT OR REPLACE INTO kb_meta(key, value) VALUES ('model_tag', ?1)",
        params![tag],
    )?;
    Ok(())
}

fn js_opts_to_opts(js: JsSearchOptions) -> SearchOptions {
    let aggregate = match js.aggregate.as_deref() {
        Some("sum") => AggregateMode::Sum,
        Some("top2sum") => AggregateMode::Top2Sum,
        _ => AggregateMode::Max,
    };
    let syntax = match js.syntax.as_deref() {
        Some("fielded") => QuerySyntax::Fielded,
        Some("raw") => QuerySyntax::Raw,
        _ => QuerySyntax::Text,
    };
    SearchOptions {
        top_k: js.top_k.map(|v| v as usize).unwrap_or(50),
        top_n: js.top_n.map(|v| v as usize).unwrap_or(5),
        rrf_k: js.rrf_k.map(|v| v as f64).unwrap_or(60.0),
        aggregate,
        filter_doc_types: js.filter.as_ref().and_then(|f| f.doc_type.clone()).unwrap_or_default(),
        filter_paths: js.filter.as_ref().and_then(|f| f.paths.clone()).unwrap_or_default(),
        syntax,
        max_chars_per_chunk: js.max_chars_per_chunk.map(|v| v as usize).unwrap_or(800),
        include_text: js.include_text.unwrap_or(true),
        require_vector: js.require_vector.unwrap_or(false),
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}
impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
