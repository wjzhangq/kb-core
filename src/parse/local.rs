use std::panic;
use std::path::Path;
use anyhow::{Context, Result};

use crate::config::ProcessingConfig;
use crate::parse::{BlockType, Okf, OkfBlock, ParsedBy};

/// Route a file to the appropriate local extractor.
pub fn extract_local(path: &Path, doc_id: i64, cfg: &ProcessingConfig) -> Result<Okf> {
    let result = panic::catch_unwind(|| {
        extract_local_inner(path, doc_id, cfg)
    });

    match result {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!("local extractor panicked for {:?}", path)),
    }
}

fn extract_local_inner(path: &Path, doc_id: i64, cfg: &ProcessingConfig) -> Result<Okf> {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Check deny list
    let deny = cfg.attachment_deny_list.iter().any(|d| {
        d.trim_start_matches('.').eq_ignore_ascii_case(ext.trim_start_matches('.'))
    });
    if deny {
        return Err(anyhow::anyhow!("file type .{ext} is in attachment deny list"));
    }

    // Check file size
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("stat {:?}", path))?;
    if metadata.len() > cfg.max_file_size_bytes {
        return Err(anyhow::anyhow!(
            "file {:?} ({} bytes) exceeds maxFileSizeBytes ({})",
            path, metadata.len(), cfg.max_file_size_bytes
        ));
    }

    let blocks = match ext.as_str() {
        "md" | "markdown" => extract_text_file(path)?,
        "txt" => extract_text_file(path)?,
        "eml" => extract_eml(path)?,
        "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" |
        "cs" | "rb" | "php" | "swift" | "kt" | "toml" | "yaml" | "yml" |
        "json" | "xml" | "html" | "htm" | "css" | "sh" | "bash" | "sql" => {
            extract_code_file(path, &ext)?
        },
        "docx" => extract_docx(path)?,
        "pdf" => extract_pdf_text_layer(path)?,
        _ => extract_text_file(path).unwrap_or_else(|_| {
            vec![OkfBlock {
                block_id: 0,
                block_type: BlockType::Para,
                text: format!("[binary file: {}]", path.display()),
                description: None,
                page: None,
                bbox: None,
                from_image: false,
            }]
        }),
    };

    Ok(Okf {
        doc_id,
        source_path: path.to_string_lossy().into_owned(),
        parsed_by: ParsedBy::Local,
        blocks,
        outline: None,
    })
}

fn extract_text_file(path: &Path) -> Result<Vec<OkfBlock>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read {:?}", path))?;
    let blocks = split_text_into_blocks(&content);
    Ok(blocks)
}

fn split_text_into_blocks(text: &str) -> Vec<OkfBlock> {
    // Split on double newlines to get logical paragraphs/sections
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    paragraphs.iter().enumerate()
        .filter_map(|(i, para)| {
            let trimmed = para.trim();
            if trimmed.is_empty() { return None; }
            let block_type = if trimmed.starts_with('#') {
                BlockType::Heading
            } else {
                BlockType::Para
            };
            Some(OkfBlock {
                block_id: i as u32,
                block_type,
                text: trimmed.to_string(),
                description: None,
                page: None,
                bbox: None,
                from_image: false,
            })
        })
        .collect()
}

fn extract_code_file(path: &Path, _ext: &str) -> Result<Vec<OkfBlock>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read {:?}", path))?;
    Ok(vec![OkfBlock {
        block_id: 0,
        block_type: BlockType::Code,
        text: content,
        description: None,
        page: None,
        bbox: None,
        from_image: false,
    }])
}

fn extract_eml(path: &Path) -> Result<Vec<OkfBlock>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read eml {:?}", path))?;
    // Minimal EML: split headers and body
    let body = content.split_once("\n\n").map(|x| x.1).unwrap_or(&content);
    let blocks = split_text_into_blocks(body);
    Ok(if blocks.is_empty() {
        vec![OkfBlock {
            block_id: 0,
            block_type: BlockType::Para,
            text: body.trim().to_string(),
            description: None,
            page: None,
            bbox: None,
            from_image: false,
        }]
    } else {
        blocks
    })
}

fn extract_docx(path: &Path) -> Result<Vec<OkfBlock>> {
    // Minimal: read as ZIP, extract word/document.xml
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .context("open docx as zip")?;
    let mut xml_content = String::new();
    zip.by_name("word/document.xml")
        .context("find word/document.xml in docx")?
        .read_to_string(&mut xml_content)?;
    // Strip XML tags to get plain text
    let text = strip_xml_tags(&xml_content);
    Ok(split_text_into_blocks(&text))
}

fn strip_xml_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => { in_tag = false; result.push(' '); }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

fn extract_pdf_text_layer(_path: &Path) -> Result<Vec<OkfBlock>> {
    // PDF text-layer extraction: return a single para block with raw bytes placeholder.
    // Full PDF extraction requires pdfium; for now return a marker that triggers remote parse.
    Err(anyhow::anyhow!("PDF requires remote parse or pdfium integration"))
}
