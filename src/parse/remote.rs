use std::time::{Duration, Instant};
use anyhow::Result;
use parking_lot::Mutex;
use reqwest::Client;
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

        let result = self.do_parse(path, doc_id).await;

        match &result {
            Ok(_) => self.record_success(),
            Err(_) => self.record_failure(),
        }

        result
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

    async fn do_parse(&self, path: &std::path::Path, doc_id: i64) -> Result<Okf> {
        let file_bytes = tokio::fs::read(path).await?;
        let filename = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename)
            .mime_str("application/octet-stream")?;

        let options_part = reqwest::multipart::Part::text(
            serde_json::json!({"textLayerThreshold": self.config.text_layer_threshold}).to_string()
        ).mime_str("application/json")?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .part("options", options_part);

        let url = format!("{}/v1/parse", self.config.endpoint);
        let resp = self.client.post(&url).multipart(form).send().await?;

        let status = resp.status();
        if status.is_client_error() {
            // 4xx: don't count as breaker failure
            return Err(anyhow::anyhow!("remote parse 4xx: {}", status));
        }
        if !status.is_success() {
            return Err(anyhow::anyhow!("remote parse error: {}", status));
        }

        let body: RemoteParseResponse = resp.json().await?;
        let blocks = body.okf.blocks.into_iter().map(|b| OkfBlock {
            block_id: b.block_id,
            block_type: parse_block_type(&b.r#type),
            text: b.text,
            description: b.description,
            page: b.page,
            bbox: b.bbox.map(|v| {
                let a: Vec<f32> = v;
                [a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0),
                 a.get(2).copied().unwrap_or(0.0), a.get(3).copied().unwrap_or(0.0)]
            }),
            from_image: b.from_image,
        }).collect();

        let outline = body.okf.outline.map(|nodes| nodes.into_iter().map(convert_outline_node).collect());

        Ok(Okf {
            doc_id,
            source_path: path.to_string_lossy().into_owned(),
            parsed_by: ParsedBy::Remote,
            blocks,
            outline,
        })
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.state.lock(), BreakerState::Open { .. })
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
    okf: RemoteOkf,
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
