# Data Model: kb-core 召回引擎

## SQLite Schema

### `kb_meta` — 知识库元信息

```sql
CREATE TABLE IF NOT EXISTS kb_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- 初始值
-- key='embedding_model'  value='multilingual-e5-small'
-- key='embedding_dim'    value='384'
-- key='embedding_quant'  value='int8'
-- key='model_tag'        value=sha256('multilingual-e5-small|384|int8')[..16]
-- key='schema_version'   value='5'
```

---

### `documents` — 文档级状态机

```sql
CREATE TABLE IF NOT EXISTS documents (
  doc_id      INTEGER PRIMARY KEY AUTOINCREMENT,
  path        TEXT NOT NULL UNIQUE,   -- 绝对路径 或 mbox:// URI
  title       TEXT,
  doc_type    TEXT NOT NULL,          -- 'text' | 'pdf' | 'pptx' | 'docx' | 'image' | 'email'
  status      TEXT NOT NULL DEFAULT 'pending_parse',
    -- pending_parse | parsing | parsed | indexed | parse_failed
  parsed_by   TEXT,                  -- 'local' | 'remote'（是否出网）
  error       TEXT,                  -- 失败原因
  added_at    INTEGER NOT NULL,      -- Unix ms
  updated_at  INTEGER NOT NULL
);
```

**状态机转换**:
```
pending_parse → parsing → parsed → indexed
                       ↘ parse_failed
```

---

### `blocks` — okf 块清单（来源 meta 真源）

```sql
CREATE TABLE IF NOT EXISTS blocks (
  doc_id      INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
  block_id    INTEGER NOT NULL,
  type        TEXT NOT NULL,         -- heading | para | list | table | code | image_ocr
  page        INTEGER,               -- PDF/PPT 页码（1-based）
  bbox        TEXT,                  -- JSON [x, y, w, h]，远程解析给
  from_image  INTEGER NOT NULL DEFAULT 0,  -- 0/1
  lin_start   INTEGER NOT NULL,      -- 该块在派生线性文本中的起始字符偏移
  lin_end     INTEGER NOT NULL,      -- 该块在派生线性文本中的结束字符偏移（exclusive）
  PRIMARY KEY (doc_id, block_id)
);

CREATE INDEX IF NOT EXISTS idx_blocks_span ON blocks(doc_id, lin_start, lin_end);
```

---

### `chunks` — 文档切分单元

```sql
CREATE TABLE IF NOT EXISTS chunks (
  chunk_id    INTEGER PRIMARY KEY AUTOINCREMENT,
  doc_id      INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
  chunk_seq   INTEGER NOT NULL,      -- 块内序号（0-based）
  text        TEXT NOT NULL,         -- chunk 原文（派生线性文本切片）
  char_start  INTEGER NOT NULL DEFAULT 0,  -- 派生线性文本坐标（起）
  char_end    INTEGER NOT NULL DEFAULT 0,  -- 派生线性文本坐标（终，exclusive）
  token_count INTEGER NOT NULL DEFAULT 0,
  truncated   INTEGER NOT NULL DEFAULT 0,  -- 0/1，超长截断标记
  embed_status INTEGER NOT NULL DEFAULT 0,
    -- 0=pending | 1=done | 2=failed | 3=skipped
  UNIQUE(doc_id, chunk_seq)
);

CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);
CREATE INDEX IF NOT EXISTS idx_chunks_embed_pending ON chunks(embed_status) WHERE embed_status = 0;
```

---

### `chunks_vec` — 向量虚表（sqlite-vec）

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(
  chunk_id   INTEGER PRIMARY KEY,
  embedding  float[384],             -- multilingual-e5-small int8, 384 维
  +doc_type  TEXT,
  +model_tag TEXT                    -- 用于跨模型污染检测
);
```

> 只存 `embed_status=1`（done）的 chunk。`binary` 量化列已移除（PRD §9）。

---

### `tantivy_meta`（磁盘上独立目录）

tantivy 索引存储于 `{dataDir}/tantivy/`，不在 SQLite 中。索引字段（chunk 级）：
- `chunk_id`（u64，FAST，stored）
- `doc_id`（u64，FAST，stored）
- `doc_type`（TEXT，stored）
- `text`（TEXT，INDEXED，stored）
- `title`（TEXT，INDEXED，stored）
- `path`（TEXT，stored）

---

## Rust 类型定义

### KBConfig

```rust
pub struct KBConfig {
    pub data_dir: String,
    pub inference: InferenceConfig,
    pub system: SystemConfig,
    pub processing: ProcessingConfig,
}

pub enum InferenceConfig {
    Bm25Only,
    LocalFirst {
        model: Option<EmbeddingModelSpec>,   // 默认 multilingual-e5-small/384/int8
        models_dir: Option<String>,
        parse: Option<RemoteParseConfig>,
    },
    Remote {
        model: EmbeddingModelSpec,
        embed_endpoint: String,
        parse: Option<RemoteParseConfig>,
    },
}

pub struct EmbeddingModelSpec {
    pub name: String,           // 默认 "multilingual-e5-small"
    pub dim: usize,             // 默认 384
    pub quantization: String,   // 默认 "int8"
}

pub struct SystemConfig {
    pub max_cpu_threads: usize,    // 默认 2
    pub low_thread_priority: bool, // 默认 true（不可逆）
    pub temp_security: TempSecurity,
}

pub enum TempSecurity {
    SecureTemp,       // 默认
    AclRestricted,    // Windows：额外剥离 Everyone/Guests ACE
}

pub struct ProcessingConfig {
    pub chunk_max_tokens: usize,           // 默认 320
    pub chunk_overlap_sentences: usize,    // 默认 2
    pub embed_batch_size: usize,           // 默认 16
    pub parse_concurrency: usize,          // 默认 4
    pub reader_reload_interval_ms: u64,    // 默认 5000
}
```

### RemoteParseConfig

```rust
pub struct RemoteParseConfig {
    pub endpoint: String,
    pub allow_remote: bool,                          // 默认 true
    pub text_layer_threshold: f32,                   // 默认 0.8
    pub on_remote_parse_unavailable: RemoteParseUnavailablePolicy,
    pub timeout_ms: u64,                             // 默认 30000
    pub headers: HashMap<String, String>,
    pub breaker: BreakerConfig,
}

pub enum RemoteParseUnavailablePolicy {
    Wait,     // 默认：熔断 open 时排队
    TextOnly, // 降级：只索引能提取的文本层
    Skip,     // 跳过图片类文档
}
```

### OkfBlock / Okf

```rust
pub struct OkfBlock {
    pub block_id: u32,
    pub block_type: BlockType,   // Heading | Para | List | Table | Code | ImageOcr
    pub text: String,
    pub page: Option<u32>,
    pub bbox: Option<[f32; 4]>,  // [x, y, w, h]
    pub from_image: bool,
}

pub struct Okf {
    pub doc_id: i64,
    pub source_path: String,
    pub parsed_by: ParsedBy,     // Local | Remote
    pub parsed_at: String,       // ISO 8601
    pub blocks: Vec<OkfBlock>,
}
```

### SearchResult（TypeScript 侧，由 napi-rs 导出）

```typescript
export interface SearchResult {
  docId: number
  path: string                // 原始文件绝对路径 或 mbox:// URI
  title?: string
  score: number
  chunks: ChunkResult[]
}

export interface ChunkResult {
  chunkId: number
  text: string
  truncated: boolean
  charOffset: [number, number]      // [start, end) 派生线性文本坐标
  pageRange?: [number, number]      // [first, last] 页码（1-based）
  bbox?: Array<{ page: number; rect: [number, number, number, number] }>
  blockTypes: string[]              // 覆盖的块类型
  fromImage: boolean
  matchedBy: Array<'bm25' | 'vector'>
  score: number
}

export interface SearchResponse {
  results: SearchResult[]
  timing: SearchTiming
  mode: 'bm25-only' | 'bm25+vec'
  vectorCoverage: number           // 0.0–1.0
  degraded?: { reason: string }
}

export interface SearchTiming {
  parseMs: number
  bm25Ms: number
  embedMs: number
  vecMs: number
  rrfMs: number
  aggregateMs: number
  totalMs: number
}
```

## 状态枚举

### DocStatus

```
pending_parse → parsing → parsed → indexed
                       ↘ parse_failed
```

### EmbedStatus

```
0 = pending   (parsed，awaiting embed queue)
1 = done      (embedding 完成，写入 chunks_vec)
2 = failed    (推理失败，仍可 BM25 命中)
3 = skipped   (bm25-only 模式)
```

## 关键约束

- `model_tag` = `sha256("{name}|{dim}|{quantization}")[..16]`；换模型必须重建 `chunks_vec`，不污染旧数据。
- `chunks_vec` 只存 `embed_status=1` 的条目；向量删除通过 `chunk_id` 精确删除。
- chunk 切分在派生线性文本上进行，优先在块边界（`lin_end`）处断开，`chunk_overlap_sentences` 在块内生效。
- 派生线性文本 = `blocks[].text` 按序拼接，块间分隔符为 `\n\n`（固定，写入 `lin_start/lin_end` 时计入）。
- `idx_blocks_span`：chunk 的 `[char_start, char_end)` 与 `blocks(lin_start, lin_end)` 区间相交查询，得到覆盖块集合。
