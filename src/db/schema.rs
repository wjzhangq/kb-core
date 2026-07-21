pub const DOCUMENTS: &str = "documents";
pub const CHUNKS: &str = "chunks";
pub const CHUNKS_VEC: &str = "chunks_vec";
pub const BLOCKS: &str = "blocks";
pub const KB_META: &str = "kb_meta";

// documents columns
pub const DOC_ID: &str = "doc_id";
pub const PATH: &str = "path";
pub const TITLE: &str = "title";
pub const DOC_TYPE: &str = "doc_type";
pub const STATUS: &str = "status";
pub const PARSED_BY: &str = "parsed_by";
pub const ERROR: &str = "error";
pub const ADDED_AT: &str = "added_at";
pub const UPDATED_AT: &str = "updated_at";

// chunks columns
pub const CHUNK_ID: &str = "chunk_id";
pub const CHUNK_SEQ: &str = "chunk_seq";
pub const TEXT: &str = "text";
pub const CHAR_START: &str = "char_start";
pub const CHAR_END: &str = "char_end";
pub const TOKEN_COUNT: &str = "token_count";
pub const TRUNCATED: &str = "truncated";
pub const EMBED_STATUS: &str = "embed_status";

// blocks columns
pub const BLOCK_ID: &str = "block_id";
pub const BLOCK_TYPE: &str = "type";
pub const PAGE: &str = "page";
pub const BBOX: &str = "bbox";
pub const FROM_IMAGE: &str = "from_image";
pub const LIN_START: &str = "lin_start";
pub const LIN_END: &str = "lin_end";
pub const DESCRIPTION: &str = "description";

// embed_status values
pub const EMBED_PENDING: i64 = 0;
pub const EMBED_DONE: i64 = 1;
pub const EMBED_FAILED: i64 = 2;
pub const EMBED_SKIPPED: i64 = 3;

// document status values
pub const DOC_STATUS_PENDING_PARSE: &str = "pending_parse";
pub const DOC_STATUS_PARSING: &str = "parsing";
pub const DOC_STATUS_PARSED: &str = "parsed";
pub const DOC_STATUS_INDEXED: &str = "indexed";
pub const DOC_STATUS_PARSE_FAILED: &str = "parse_failed";
