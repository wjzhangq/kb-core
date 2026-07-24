use std::time::{Duration, Instant};
use anyhow::Result;
use parking_lot::Mutex;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::config::RemoteParseConfig;
use crate::parse::{BlockType, Okf, OkfBlock, ParsedBy};

// ── Three-state breaker ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum BreakerState {
    Closed,
    Open { opened_at: Instant },
    HalfOpen,
}

/// Internal parse error split by whether it should trip the circuit breaker.
///
/// Only genuine service outages (`Service`) count toward the breaker. Business
/// outcomes (`Business`) — a rejected file, a `parse_failed` task, an expired
/// task id — mean the service is healthy and answered; they must never open the
/// breaker, or a handful of bad documents would take remote parse down for all
/// good documents (prd-v7 §3.3, §3.4).
enum ParseError {
    Service(anyhow::Error),
    Business(anyhow::Error),
}

/// Outcome of classifying an HTTP status: either the response body is worth
/// reading, or the status maps directly to a `ParseError`.
enum StatusClass {
    /// 2xx — proceed to read the body.
    Ok,
    Err(ClassifiedErr),
}

enum ClassifiedErr {
    Service,
    Business,
}

pub struct RemoteParser {
    config: RemoteParseConfig,
    client: Client,
    state: Mutex<BreakerState>,
    consecutive_failures: Mutex<u32>,
}

impl RemoteParser {
    pub fn new(config: RemoteParseConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                for (k, v) in &config.headers {
                    if let (Ok(name), Ok(value)) = (
                        reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                        reqwest::header::HeaderValue::from_str(v),
                    ) {
                        headers.insert(name, value);
                    }
                }
                headers
            })
            .build()
            .expect("build reqwest client");

        RemoteParser {
            config,
            client,
            state: Mutex::new(BreakerState::Closed),
            consecutive_failures: Mutex::new(0),
        }
    }

    pub async fn parse_file(&self, path: &std::path::Path, doc_id: i64) -> Result<Okf> {
        self.check_breaker()?;

        match self.do_parse(path, doc_id).await {
            Ok(okf) => {
                self.record_success();
                Ok(okf)
            }
            Err(ParseError::Service(e)) => {
                self.record_failure();
                Err(e)
            }
            // Business failures leave breaker state untouched: the service is
            // healthy, this document simply could not be parsed.
            Err(ParseError::Business(e)) => Err(e),
        }
    }

    fn check_breaker(&self) -> Result<()> {
        let mut state = self.state.lock();
        match &*state {
            BreakerState::Closed => Ok(()),
            BreakerState::HalfOpen => Ok(()),
            BreakerState::Open { opened_at } => {
                let reset = Duration::from_millis(self.config.breaker.reset_timeout_ms);
                if opened_at.elapsed() >= reset {
                    *state = BreakerState::HalfOpen;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("remote parse circuit breaker is OPEN"))
                }
            }
        }
    }

    fn record_success(&self) {
        *self.state.lock() = BreakerState::Closed;
        *self.consecutive_failures.lock() = 0;
    }

    fn record_failure(&self) {
        let mut failures = self.consecutive_failures.lock();
        *failures += 1;
        if *failures >= self.config.breaker.failure_threshold {
            *self.state.lock() = BreakerState::Open { opened_at: Instant::now() };
        }
    }

    async fn do_parse(&self, path: &std::path::Path, doc_id: i64) -> std::result::Result<Okf, ParseError> {
        let file_bytes = tokio::fs::read(path).await
            .map_err(|e| ParseError::Service(anyhow::anyhow!("read {:?}: {e}", path)))?;
        let filename = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| ParseError::Service(e.into()))?;

        let options = serde_json::json!({
            "textLayerThreshold": self.config.text_layer_threshold,
            "enableImageDescription": self.config.enable_image_description,
            "enableOutline": self.config.enable_outline,
        });
        let options_part = reqwest::multipart::Part::text(options.to_string())
            .mime_str("application/json")
            .map_err(|e| ParseError::Service(e.into()))?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .part("options", options_part);

        let url = format!("{}/v1/parse", self.config.endpoint);
        let resp = self.client.post(&url).multipart(form).send().await
            .map_err(|e| ParseError::Service(anyhow::anyhow!("remote parse POST failed: {e}")))?;

        let status = resp.status();

        // 202: server auto-converted to async — switch to polling.
        if status == StatusCode::ACCEPTED {
            let accepted: TaskAccepted = resp.json().await
                .map_err(|e| ParseError::Service(anyhow::anyhow!("decode 202 body: {e}")))?;
            return self.poll_task(&accepted.task_id, accepted.retry_after_ms, path, doc_id).await;
        }

        match classify_post_status(status) {
            StatusClass::Ok => {}
            StatusClass::Err(ClassifiedErr::Business) => {
                return Err(ParseError::Business(anyhow::anyhow!("remote parse rejected ({status})")));
            }
            StatusClass::Err(ClassifiedErr::Service) => {
                return Err(ParseError::Service(anyhow::anyhow!("remote parse error: {status}")));
            }
        }

        let body: RemoteParseResponse = resp.json().await
            .map_err(|e| ParseError::Service(anyhow::anyhow!("decode parse body: {e}")))?;
        let okf = body.okf.ok_or_else(|| {
            ParseError::Service(anyhow::anyhow!("200 response missing okf"))
        })?;
        Ok(okf_from_wire(okf, path, doc_id))
    }

    /// Poll `GET /v1/tasks/{taskId}` until the task terminates or the overall
    /// budget is exhausted. Exponential backoff (init → cap), preferring any
    /// `retryAfterMs` the server hands back (prd-v7 §3.3).
    async fn poll_task(
        &self,
        task_id: &str,
        first_retry_after_ms: Option<u64>,
        path: &std::path::Path,
        doc_id: i64,
    ) -> std::result::Result<Okf, ParseError> {
        let url = format!("{}/v1/tasks/{}", self.config.endpoint, task_id);
        let deadline = Instant::now() + Duration::from_millis(self.config.async_poll_budget_ms);
        let mut backoff_ms = self.config.async_poll_initial_ms;
        // The 202 body may already suggest a retry interval for the first poll.
        let mut next_wait_ms = first_retry_after_ms.unwrap_or(backoff_ms);

        loop {
            if Instant::now() >= deadline {
                return Err(ParseError::Business(anyhow::anyhow!(
                    "async parse task {task_id} exceeded poll budget"
                )));
            }
            tokio::time::sleep(Duration::from_millis(next_wait_ms)).await;

            let resp = self.client.get(&url).send().await
                .map_err(|e| ParseError::Service(anyhow::anyhow!("poll GET failed: {e}")))?;
            let status = resp.status();

            match classify_get_status(status) {
                StatusClass::Ok => {}
                StatusClass::Err(ClassifiedErr::Business) => {
                    // 404/410 — task id unknown or result expired.
                    return Err(ParseError::Business(anyhow::anyhow!(
                        "async parse task {task_id} unavailable ({status})"
                    )));
                }
                StatusClass::Err(ClassifiedErr::Service) => {
                    return Err(ParseError::Service(anyhow::anyhow!(
                        "poll error for task {task_id}: {status}"
                    )));
                }
            }

            let body: TaskStatus = resp.json().await
                .map_err(|e| ParseError::Service(anyhow::anyhow!("decode task status: {e}")))?;

            match body.status.as_str() {
                "succeeded" => {
                    let okf = body.okf.ok_or_else(|| {
                        ParseError::Service(anyhow::anyhow!("succeeded task missing okf"))
                    })?;
                    return Ok(okf_from_wire(okf, path, doc_id));
                }
                // A failed task is a business result delivered over HTTP 200,
                // NOT a service outage — must not trip the breaker (prd-v7 §3.3).
                "failed" => {
                    let msg = body.error
                        .map(|e| e.message)
                        .unwrap_or_else(|| "parse_failed".into());
                    return Err(ParseError::Business(anyhow::anyhow!(
                        "async parse task {task_id} failed: {msg}"
                    )));
                }
                // queued / layout / vision / assemble / running / pending → keep polling.
                _ => {
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(self.config.async_poll_max_ms);
                    next_wait_ms = body.retry_after_ms.unwrap_or(backoff_ms);
                }
            }
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.state.lock(), BreakerState::Open { .. })
    }
}

// ── Status classification (pure, unit-tested) ───────────────────────────────

/// Classify a `POST /v1/parse` response status. 202 is handled by the caller
/// before this is reached.
fn classify_post_status(status: StatusCode) -> StatusClass {
    if status.is_success() {
        StatusClass::Ok
    } else if status.is_client_error() {
        // 400/422/429 — bad file, parse_failed (sync), queue_full. Not breaker.
        StatusClass::Err(ClassifiedErr::Business)
    } else {
        // 5xx — service internal error / docling-serve unreachable. Breaker.
        StatusClass::Err(ClassifiedErr::Service)
    }
}

/// Classify a `GET /v1/tasks/{taskId}` response status. A GET 5xx means the
/// query action itself failed (breaker); 404/410 are business (unknown/expired).
fn classify_get_status(status: StatusCode) -> StatusClass {
    if status.is_success() {
        StatusClass::Ok
    } else if status.is_client_error() {
        StatusClass::Err(ClassifiedErr::Business)
    } else {
        StatusClass::Err(ClassifiedErr::Service)
    }
}

// ── Wire → domain conversion ────────────────────────────────────────────────

fn okf_from_wire(okf: RemoteOkf, path: &std::path::Path, doc_id: i64) -> Okf {
    let blocks = okf.blocks.into_iter().map(|b| OkfBlock {
        block_id: b.block_id,
        block_type: parse_block_type(&b.r#type),
        text: b.text,
        description: b.description,
        page: b.page,
        bbox: b.bbox.map(|v| {
            [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0),
             v.get(2).copied().unwrap_or(0.0), v.get(3).copied().unwrap_or(0.0)]
        }),
        from_image: b.from_image,
    }).collect();

    let outline = okf.outline.map(|nodes| nodes.into_iter().map(convert_outline_node).collect());

    Okf {
        doc_id,
        source_path: path.to_string_lossy().into_owned(),
        parsed_by: ParsedBy::Remote,
        blocks,
        outline,
    }
}

fn parse_block_type(s: &str) -> BlockType {
    match s {
        "heading" => BlockType::Heading,
        "list" => BlockType::List,
        "table" => BlockType::Table,
        "code" => BlockType::Code,
        "image_ocr" => BlockType::ImageOcr,
        "image_caption" => BlockType::ImageCaption,
        "outline_heading" => BlockType::OutlineHeading,
        _ => BlockType::Para,
    }
}

fn convert_outline_node(n: RemoteOutlineNode) -> crate::parse::OutlineNode {
    crate::parse::OutlineNode {
        title: n.title,
        page: n.page,
        level: n.level,
        block_id: n.block_id,
        children: n.children.unwrap_or_default().into_iter().map(convert_outline_node).collect(),
    }
}

// ── Wire types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RemoteParseResponse {
    okf: Option<RemoteOkf>,
}

/// 202 Accepted body when the server auto-converts to async.
#[derive(Deserialize)]
struct TaskAccepted {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "retryAfterMs", default)]
    retry_after_ms: Option<u64>,
}

/// `GET /v1/tasks/{taskId}` body: status + (on success) okf, or (on failure) error.
#[derive(Deserialize)]
struct TaskStatus {
    status: String,
    #[serde(default)]
    okf: Option<RemoteOkf>,
    #[serde(default)]
    error: Option<TaskError>,
    #[serde(rename = "retryAfterMs", default)]
    retry_after_ms: Option<u64>,
}

#[derive(Deserialize)]
struct TaskError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct RemoteOkf {
    blocks: Vec<RemoteBlock>,
    outline: Option<Vec<RemoteOutlineNode>>,
}

#[derive(Deserialize)]
struct RemoteBlock {
    #[serde(rename = "blockId")]
    block_id: u32,
    #[serde(rename = "type")]
    r#type: String,
    text: String,
    #[serde(default)]
    description: Option<String>,
    page: Option<u32>,
    bbox: Option<Vec<f32>>,
    #[serde(rename = "fromImage", default)]
    from_image: bool,
}

#[derive(Deserialize)]
struct RemoteOutlineNode {
    title: String,
    page: Option<u32>,
    level: u32,
    #[serde(rename = "blockId")]
    block_id: Option<u32>,
    children: Option<Vec<RemoteOutlineNode>>,
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn is_service(c: StatusClass) -> bool {
        matches!(c, StatusClass::Err(ClassifiedErr::Service))
    }
    fn is_business(c: StatusClass) -> bool {
        matches!(c, StatusClass::Err(ClassifiedErr::Business))
    }
    fn is_ok(c: StatusClass) -> bool {
        matches!(c, StatusClass::Ok)
    }

    #[test]
    fn post_2xx_is_ok() {
        assert!(is_ok(classify_post_status(StatusCode::OK)));
    }

    #[test]
    fn post_4xx_is_business_not_breaker() {
        // 400 unsupported_file_type / file_too_large, 422 parse_failed, 429 queue_full
        for code in [400u16, 413, 422, 429] {
            assert!(
                is_business(classify_post_status(StatusCode::from_u16(code).unwrap())),
                "status {code} should be business"
            );
        }
    }

    #[test]
    fn post_5xx_is_service_breaker() {
        for code in [500u16, 502, 503] {
            assert!(
                is_service(classify_post_status(StatusCode::from_u16(code).unwrap())),
                "status {code} should be service"
            );
        }
    }

    #[test]
    fn get_404_410_is_business() {
        assert!(is_business(classify_get_status(StatusCode::NOT_FOUND)));
        assert!(is_business(classify_get_status(StatusCode::GONE)));
    }

    #[test]
    fn get_5xx_is_service() {
        assert!(is_service(classify_get_status(StatusCode::INTERNAL_SERVER_ERROR)));
        assert!(is_service(classify_get_status(StatusCode::SERVICE_UNAVAILABLE)));
    }

    /// Exponential backoff: init 2s, double each idle poll, cap at 15s.
    #[test]
    fn backoff_doubles_then_caps() {
        let init = 2_000u64;
        let cap = 15_000u64;
        let mut b = init;
        let mut seq = vec![b];
        for _ in 0..5 {
            b = (b.saturating_mul(2)).min(cap);
            seq.push(b);
        }
        // 2000, 4000, 8000, 15000 (capped from 16000), 15000, 15000
        assert_eq!(seq, vec![2_000, 4_000, 8_000, 15_000, 15_000, 15_000]);
    }

    /// A server-provided retryAfterMs overrides the computed backoff.
    #[test]
    fn retry_after_overrides_backoff() {
        let computed_backoff = 8_000u64;
        let server_hint: Option<u64> = Some(5_000);
        let next = server_hint.unwrap_or(computed_backoff);
        assert_eq!(next, 5_000);

        let no_hint: Option<u64> = None;
        assert_eq!(no_hint.unwrap_or(computed_backoff), 8_000);
    }
}
