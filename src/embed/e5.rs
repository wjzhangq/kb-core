use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use fastembed::{InitOptionsUserDefined, TextEmbedding, UserDefinedEmbeddingModel};

use crate::config::EmbeddingModelSpec;

pub struct E5Engine {
    model: TextEmbedding,
}

impl E5Engine {
    pub fn new(
        spec: &EmbeddingModelSpec,
        models_dir: &Path,
        max_cpu_threads: usize,
    ) -> Result<Self> {
        let model_path = models_dir.join(&spec.name);

        if !model_path.exists() {
            return Err(anyhow::anyhow!(
                "ModelNotFoundError: model '{}' not found at {:?}",
                spec.name, model_path
            ));
        }

        let onnx_file = format!("model_{}.onnx", spec.quantization);
        let onnx_path = model_path.join(&onnx_file);
        let onnx_path = if onnx_path.exists() {
            onnx_path
        } else {
            model_path.join("model_quantized.onnx")
        };

        let tokenizer_path = model_path.join("tokenizer.json");

        if !onnx_path.exists() {
            return Err(anyhow::anyhow!(
                "ModelNotFoundError: ONNX file not found at {:?}", onnx_path
            ));
        }
        if !tokenizer_path.exists() {
            return Err(anyhow::anyhow!(
                "ModelNotFoundError: tokenizer.json not found at {:?}", tokenizer_path
            ));
        }

        let mut additional = vec![];
        let config_path = model_path.join("tokenizer_config.json");
        if config_path.exists() { additional.push(config_path); }
        let special_path = model_path.join("special_tokens_map.json");
        if special_path.exists() { additional.push(special_path); }

        let user_model = UserDefinedEmbeddingModel::new(onnx_path, tokenizer_path)
            .with_additional_files(additional);

        let opts = InitOptionsUserDefined {
            model_code: "kb-core/multilingual-e5-small".into(),
            ..Default::default()
        };

        let model = TextEmbedding::try_new_from_user_defined(user_model, opts)
            .context("initialize fastembed TextEmbedding")?;

        Ok(E5Engine { model })
    }

    /// Embed passages with the required "passage: " prefix.
    pub fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts.iter()
            .map(|t| format!("passage: {}", t))
            .collect();
        let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
        let embeddings = self.model.embed(refs, None)
            .context("fastembed passage embedding")?;
        Ok(embeddings)
    }

    /// Embed a query with the required "query: " prefix.
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("query: {}", text);
        let mut embeddings = self.model.embed(vec![prefixed.as_str()], None)
            .context("fastembed query embedding")?;
        embeddings.pop().ok_or_else(|| anyhow::anyhow!("embed returned empty result"))
    }
}
