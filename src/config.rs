use napi_derive::napi;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Inference configuration ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingModelSpec {
    pub name: String,
    pub dim: usize,
    pub quantization: String,
}

impl Default for EmbeddingModelSpec {
    fn default() -> Self {
        Self {
            name: "multilingual-e5-small".into(),
            dim: 384,
            quantization: "int8".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum InferenceConfig {
    Bm25Only,
    LocalFirst {
        model: EmbeddingModelSpec,
        models_dir: Option<String>,
        parse: Option<RemoteParseConfig>,
    },
    Remote {
        model: EmbeddingModelSpec,
        embed_endpoint: String,
        parse: Option<RemoteParseConfig>,
    },
}

impl Default for InferenceConfig {
    fn default() -> Self {
        InferenceConfig::LocalFirst {
            model: EmbeddingModelSpec::default(),
            models_dir: None,
            parse: None,
        }
    }
}

// ── Remote parse ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum RemoteParseUnavailablePolicy {
    Wait,
    TextOnly,
    Skip,
}

impl Default for RemoteParseUnavailablePolicy {
    fn default() -> Self { RemoteParseUnavailablePolicy::Wait }
}

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub failure_threshold: u32,
    pub reset_timeout_ms: u64,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self { failure_threshold: 5, reset_timeout_ms: 30_000 }
    }
}

#[derive(Debug, Clone)]
pub struct RemoteParseConfig {
    pub endpoint: String,
    pub allow_remote: bool,
    pub text_layer_threshold: f32,
    pub on_remote_parse_unavailable: RemoteParseUnavailablePolicy,
    pub timeout_ms: u64,
    pub headers: HashMap<String, String>,
    pub breaker: BreakerConfig,
}

// ── System + processing ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TempSecurity {
    SecureTemp,
    AclRestricted,
}

impl Default for TempSecurity {
    fn default() -> Self { TempSecurity::SecureTemp }
}

#[derive(Debug, Clone)]
pub struct SystemConfig {
    pub max_cpu_threads: usize,
    pub low_thread_priority: bool,
    pub temp_security: TempSecurity,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self { max_cpu_threads: 2, low_thread_priority: true, temp_security: TempSecurity::default() }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessingConfig {
    pub chunk_max_tokens: usize,
    pub chunk_overlap_sentences: usize,
    pub embed_batch_size: usize,
    pub parse_concurrency: usize,
    pub reader_reload_interval_ms: u64,
    pub max_file_size_bytes: u64,
    pub attachment_deny_list: Vec<String>,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            chunk_max_tokens: 320,
            chunk_overlap_sentences: 2,
            embed_batch_size: 16,
            parse_concurrency: 4,
            reader_reload_interval_ms: 5_000,
            max_file_size_bytes: 104_857_600,
            attachment_deny_list: vec![
                ".exe".into(), ".dll".into(), ".bat".into(),
                ".sh".into(), ".app".into(), ".zip".into(),
            ],
        }
    }
}

// ── Top-level config ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KBConfig {
    pub data_dir: String,
    pub inference: InferenceConfig,
    pub system: SystemConfig,
    pub processing: ProcessingConfig,
}

// ── JS-facing types (napi objects) ────────────────────────────────────────

#[napi(object)]
pub struct JsEmbeddingModelSpec {
    pub name: String,
    pub dim: u32,
    pub quantization: Option<String>,
}

#[napi(object)]
pub struct JsBreakerConfig {
    pub failure_threshold: Option<u32>,
    pub reset_timeout_ms: Option<f64>,
}

#[napi(object)]
pub struct JsRemoteParseConfig {
    pub endpoint: String,
    pub allow_remote: Option<bool>,
    pub text_layer_threshold: Option<f64>,
    pub on_remote_parse_unavailable: Option<String>,
    pub timeout_ms: Option<f64>,
    pub headers: Option<HashMap<String, String>>,
    pub breaker: Option<JsBreakerConfig>,
}

#[napi(object)]
pub struct JsSystemConfig {
    pub max_cpu_threads: Option<u32>,
    pub low_thread_priority: Option<bool>,
    pub temp_security: Option<String>,
}

#[napi(object)]
pub struct JsProcessingConfig {
    pub chunk_max_tokens: Option<u32>,
    pub chunk_overlap_sentences: Option<u32>,
    pub embed_batch_size: Option<u32>,
    pub parse_concurrency: Option<u32>,
    pub reader_reload_interval_ms: Option<f64>,
    pub max_file_size_bytes: Option<f64>,
    pub attachment_deny_list: Option<Vec<String>>,
}

#[napi(object)]
pub struct JsInferenceConfig {
    pub mode: String,
    pub model: Option<JsEmbeddingModelSpec>,
    pub models_dir: Option<String>,
    pub embed_endpoint: Option<String>,
    pub parse: Option<JsRemoteParseConfig>,
}

#[napi(object)]
pub struct JsKBConfig {
    pub data_dir: String,
    pub inference: Option<JsInferenceConfig>,
    pub system: Option<JsSystemConfig>,
    pub processing: Option<JsProcessingConfig>,
}

#[napi(object)]
pub struct JsAddResult {
    pub doc_id: i64,
    pub path: String,
    pub status: String,
}

#[napi(object)]
pub struct JsSearchFilter {
    pub doc_type: Option<Vec<String>>,
    pub paths: Option<Vec<String>>,
}

#[napi(object)]
pub struct JsSearchOptions {
    pub top_k: Option<u32>,
    pub top_n: Option<u32>,
    pub rrf_k: Option<u32>,
    pub aggregate: Option<String>,
    pub filter: Option<JsSearchFilter>,
    pub syntax: Option<String>,
    pub expand_synonyms: Option<bool>,
    pub max_chars_per_chunk: Option<u32>,
    pub include_text: Option<bool>,
    pub require_vector: Option<bool>,
}

#[napi(object)]
pub struct JsBbox {
    pub page: u32,
    pub rect: Vec<f64>,
}

#[napi(object)]
pub struct JsChunkResult {
    pub chunk_id: i64,
    pub text: String,
    pub truncated: bool,
    pub char_offset: Vec<i64>,
    pub page_range: Option<Vec<u32>>,
    pub bbox: Option<Vec<JsBbox>>,
    pub block_types: Vec<String>,
    pub from_image: bool,
    pub matched_by: Vec<String>,
    pub score: f64,
}

#[napi(object)]
pub struct JsSearchResult {
    pub doc_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub score: f64,
    pub chunks: Vec<JsChunkResult>,
}

#[napi(object)]
pub struct JsSearchTiming {
    pub parse_ms: f64,
    pub bm25_ms: f64,
    pub embed_ms: f64,
    pub vec_ms: f64,
    pub rrf_ms: f64,
    pub aggregate_ms: f64,
    pub total_ms: f64,
}

#[napi(object)]
pub struct JsDegraded {
    pub reason: String,
}

#[napi(object)]
pub struct JsSearchResponse {
    pub results: Vec<JsSearchResult>,
    pub timing: JsSearchTiming,
    pub mode: String,
    pub vector_coverage: f64,
    pub degraded: Option<JsDegraded>,
}

#[napi(object)]
pub struct JsStatusWarning {
    pub r#type: String,
    pub message: String,
    pub doc_ids: Option<Vec<i64>>,
}

#[napi(object)]
pub struct JsKBStatus {
    pub total: i64,
    pub pending_parse: i64,
    pub parsing: i64,
    pub parsed: i64,
    pub indexed: i64,
    pub parse_failed: i64,
    pub vector_coverage: f64,
    pub chunk_total: i64,
    pub chunk_embed_done: i64,
    pub chunk_embed_pending: i64,
    pub chunk_embed_failed: i64,
    pub wal_enabled: bool,
    pub writer_lock_held: bool,
    pub model_ready: bool,
    pub warnings: Vec<JsStatusWarning>,
}

// ── Config conversion helpers ─────────────────────────────────────────────

impl KBConfig {
    pub fn from_js(js: JsKBConfig) -> napi::Result<Self> {
        let inference = js.inference.map(|i| parse_inference(i)).transpose()?.unwrap_or_default();
        let system = js.system.map(parse_system).unwrap_or_default();
        let processing = js.processing.map(parse_processing).unwrap_or_default();
        Ok(KBConfig { data_dir: js.data_dir, inference, system, processing })
    }
}

fn parse_inference(js: JsInferenceConfig) -> napi::Result<InferenceConfig> {
    let model_spec = |m: JsEmbeddingModelSpec| EmbeddingModelSpec {
        name: m.name,
        dim: m.dim as usize,
        quantization: m.quantization.unwrap_or_else(|| "int8".into()),
    };
    match js.mode.as_str() {
        "bm25-only" => Ok(InferenceConfig::Bm25Only),
        "local-first" => Ok(InferenceConfig::LocalFirst {
            model: js.model.map(model_spec).unwrap_or_default(),
            models_dir: js.models_dir,
            parse: js.parse.map(parse_remote_parse_config),
        }),
        "remote" => {
            let endpoint = js.embed_endpoint.ok_or_else(|| {
                napi::Error::from_reason("mode=remote requires embedEndpoint")
            })?;
            let model = js.model.map(model_spec).ok_or_else(|| {
                napi::Error::from_reason("mode=remote requires model")
            })?;
            Ok(InferenceConfig::Remote {
                model, embed_endpoint: endpoint,
                parse: js.parse.map(parse_remote_parse_config),
            })
        }
        other => Err(napi::Error::from_reason(format!("Unknown inference mode: {other}"))),
    }
}

fn parse_remote_parse_config(js: JsRemoteParseConfig) -> RemoteParseConfig {
    let policy = match js.on_remote_parse_unavailable.as_deref() {
        Some("text-only") => RemoteParseUnavailablePolicy::TextOnly,
        Some("skip") => RemoteParseUnavailablePolicy::Skip,
        _ => RemoteParseUnavailablePolicy::Wait,
    };
    let breaker = js.breaker.map(|b| BreakerConfig {
        failure_threshold: b.failure_threshold.unwrap_or(5),
        reset_timeout_ms: b.reset_timeout_ms.map(|v| v as u64).unwrap_or(30_000),
    }).unwrap_or_default();
    RemoteParseConfig {
        endpoint: js.endpoint,
        allow_remote: js.allow_remote.unwrap_or(true),
        text_layer_threshold: js.text_layer_threshold.map(|v| v as f32).unwrap_or(0.8),
        on_remote_parse_unavailable: policy,
        timeout_ms: js.timeout_ms.map(|v| v as u64).unwrap_or(30_000),
        headers: js.headers.unwrap_or_default(),
        breaker,
    }
}

fn parse_system(js: JsSystemConfig) -> SystemConfig {
    let temp_security = match js.temp_security.as_deref() {
        Some("acl-restricted") => TempSecurity::AclRestricted,
        _ => TempSecurity::SecureTemp,
    };
    SystemConfig {
        max_cpu_threads: js.max_cpu_threads.map(|v| v as usize).unwrap_or(2),
        low_thread_priority: js.low_thread_priority.unwrap_or(true),
        temp_security,
    }
}

fn parse_processing(js: JsProcessingConfig) -> ProcessingConfig {
    ProcessingConfig {
        chunk_max_tokens: js.chunk_max_tokens.map(|v| v as usize).unwrap_or(320),
        chunk_overlap_sentences: js.chunk_overlap_sentences.map(|v| v as usize).unwrap_or(2),
        embed_batch_size: js.embed_batch_size.map(|v| v as usize).unwrap_or(16),
        parse_concurrency: js.parse_concurrency.map(|v| v as usize).unwrap_or(4),
        reader_reload_interval_ms: js.reader_reload_interval_ms.map(|v| v as u64).unwrap_or(5_000),
        max_file_size_bytes: js.max_file_size_bytes.map(|v| v as u64).unwrap_or(104_857_600),
        attachment_deny_list: js.attachment_deny_list.unwrap_or_else(|| {
            vec![".exe".into(), ".dll".into(), ".bat".into(), ".sh".into(), ".app".into(), ".zip".into()]
        }),
    }
}
