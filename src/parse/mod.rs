pub mod local;
pub mod remote;

/// The okf (open knowledge format) output of parsing a document.
#[derive(Debug, Clone)]
pub struct Okf {
    pub doc_id: i64,
    pub source_path: String,
    pub parsed_by: ParsedBy,
    pub blocks: Vec<OkfBlock>,
    pub outline: Option<Vec<OutlineNode>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedBy {
    Local,
    Remote,
}

impl ParsedBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParsedBy::Local => "local",
            ParsedBy::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OkfBlock {
    pub block_id: u32,
    pub block_type: BlockType,
    pub text: String,
    /// Model-generated visual description (image_ocr / image_caption only).
    pub description: Option<String>,
    pub page: Option<u32>,
    pub bbox: Option<[f32; 4]>,
    pub from_image: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockType {
    Heading,
    Para,
    List,
    Table,
    Code,
    ImageOcr,
    /// Pure visual image — text is empty, description carries the content.
    ImageCaption,
    /// Flattened outline node written as a searchable block.
    OutlineHeading,
}

impl BlockType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockType::Heading => "heading",
            BlockType::Para => "para",
            BlockType::List => "list",
            BlockType::Table => "table",
            BlockType::Code => "code",
            BlockType::ImageOcr => "image_ocr",
            BlockType::ImageCaption => "image_caption",
            BlockType::OutlineHeading => "outline_heading",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutlineNode {
    pub title: String,
    pub page: Option<u32>,
    pub level: u32,
    pub block_id: Option<u32>,
    pub children: Vec<OutlineNode>,
}

/// A parsed chunk ready for BM25 indexing.
#[derive(Debug, Clone)]
pub struct ParsedChunk {
    pub chunk_seq: i64,
    pub text: String,
    pub char_start: i64,
    pub char_end: i64,
    pub token_count: i64,
    pub truncated: bool,
}
