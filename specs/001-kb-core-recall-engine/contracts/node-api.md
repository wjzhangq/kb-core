# Node API Contract: kb-core

## 包导入

```typescript
import { KnowledgeBase } from 'kb-core'
// 或 CommonJS
const { KnowledgeBase } = require('kb-core')
```

---

## `new KnowledgeBase(options: KBConfig)`

构造知识库实例。同步返回，内部完成：SQLite 打开/迁移（005）、flock writer 锁获取、三线程池初始化、startup 清理 `{dataDir}/tmp/` 残留、模型就绪检查。

```typescript
export interface KBConfig {
  dataDir: string
  inference?: InferenceConfig      // 默认 LocalFirst
  system?: SystemConfig
  processing?: ProcessingConfig
}

export type InferenceConfig =
  | { mode: 'bm25-only' }
  | {
      mode: 'local-first'
      model?: EmbeddingModelSpec    // 默认 multilingual-e5-small/384/int8
      modelsDir?: string            // 默认 <package>/models/
      parse?: RemoteParseConfig
    }
  | {
      mode: 'remote'
      model: EmbeddingModelSpec
      embedEndpoint: string
      parse?: RemoteParseConfig
    }

export interface EmbeddingModelSpec {
  name: string          // e.g. 'multilingual-e5-small'
  dim: number           // e.g. 384
  quantization?: string // e.g. 'int8'
}

export interface SystemConfig {
  maxCpuThreads?: number            // 默认 2
  lowThreadPriority?: boolean       // 默认 true，不可逆
  tempSecurity?: 'secure-temp' | 'acl-restricted'  // 默认 'secure-temp'
}

export interface ProcessingConfig {
  chunkMaxTokens?: number           // 默认 320
  chunkOverlapSentences?: number    // 默认 2
  embedBatchSize?: number           // 默认 16
  parseConcurrency?: number         // 默认 4
  readerReloadIntervalMs?: number   // 默认 5000
  maxFileSizeBytes?: number         // 附件/上传文件大小上限，默认 104857600（100MB）
  attachmentDenyList?: string[]     // 拒绝索引的扩展名，默认 ['.exe','.dll','.bat','.sh','.app','.zip']
}

export interface RemoteParseConfig {
  endpoint: string
  allowRemote?: boolean                        // 默认 true
  textLayerThreshold?: number                  // 默认 0.8
  onRemoteParseUnavailable?: 'wait' | 'text-only' | 'skip'  // 默认 'wait'
  timeoutMs?: number                           // 默认 30000
  headers?: Record<string, string>
  breaker?: BreakerConfig
}

export interface BreakerConfig {
  failureThreshold?: number    // 默认 5
  resetTimeoutMs?: number      // 默认 30000
}
```

**Throws**:
- `KBLockError`：另一个写进程已持有 flock 锁（含 `heldBy?: string` 诊断信息）
- `KBModelMismatchError`：数据库 `model_tag` 与当前配置不符（含 `expected` / `found` 字段）
- `ModelNotFoundError`：`modelsDir` 中找不到所需模型文件（ONNX 文件缺失）
- `Error`：dataDir 不可写、SQLite 迁移失败等

---

## `kb.add(path: string | string[]): Promise<AddResult[]>`

将文档加入索引队列。**立即返回**，解析与 embedding 异步进行。

```typescript
export interface AddResult {
  docId: number
  path: string
  status: 'pending_parse' | 'already_indexed'  // 路径已存在时返回 already_indexed
}
```

**Behavior**:
- 相同路径重复调用：`INSERT OR IGNORE`，返回 `already_indexed`，不重复解析。
- `path[]` 批量调用：原子性逐条登记，全部放入解析队列后返回。
- 解析分流逻辑（见 data-model.md §分流）在后台阶段 A 执行，调用方无需关心。

---

## `kb.search(query: string, options?: SearchOptions): Promise<SearchResponse>`

检索知识库。全程零出站网络请求（索引阶段的远程解析与检索无关）。

```typescript
export interface SearchOptions {
  topK?: number              // 每路召回 chunk 数，默认 50
  topN?: number              // 返回文档数，默认 5
  rrfK?: number              // RRF 常数，默认 60
  aggregate?: 'max' | 'sum' | 'top2sum'  // chunk→doc 聚合，默认 'max'
  filter?: SearchFilter
  syntax?: 'text' | 'fielded' | 'raw'    // 默认 'text'（自动转义）
  expandSynonyms?: boolean   // 默认 false
  maxCharsPerChunk?: number  // chunk 文本截断，默认 800
  includeText?: boolean      // 默认 true；false 只返回定位，省 token
  requireVector?: boolean    // 默认 false；true 时只检索 embed_status='done' 的 chunk
  signal?: AbortSignal
}

export interface SearchFilter {
  docType?: string[]         // 只搜指定文档类型
  paths?: string[]           // 只搜指定路径前缀
}

export interface SearchResponse {
  results: SearchResult[]
  timing: SearchTiming
  mode: 'bm25-only' | 'bm25+vec'
  vectorCoverage: number            // 0.0–1.0，已 embed chunk / 总 chunk
  degraded?: { reason: string }     // 覆盖率不足 / 模型未下载 / requireVector 触发降级
}

export interface SearchResult {
  docId: number
  path: string
  title?: string
  score: number
  chunks: ChunkResult[]
}

export interface ChunkResult {
  chunkId: number
  text: string                       // 受 maxCharsPerChunk 截断
  truncated: boolean                 // 原始 chunk 是否因 token 超限被截断
  charOffset: [number, number]       // [start, end) 派生线性文本坐标
  pageRange?: [number, number]       // [firstPage, lastPage]（1-based）
  bbox?: Array<{ page: number; rect: [number, number, number, number] }>
  blockTypes: string[]               // 覆盖块类型：heading/para/table/code/image_ocr
  fromImage: boolean                 // 是否源自图片 OCR（经远程解析）
  matchedBy: Array<'bm25' | 'vector'>
  score: number
}

export interface SearchTiming {
  parseMs: number      // query 解析 + e5 前缀注入
  bm25Ms: number
  embedMs: number      // query embedding 推理
  vecMs: number        // sqlite-vec 扫描
  rrfMs: number
  aggregateMs: number  // chunk→doc 聚合
  totalMs: number
}
```

**可选字段约定**：`SearchResult.chunks` 中的 `pageRange`、`bbox` 以及 `SearchResponse` 中的 `degraded` 等标注为 `?` 的字段，在不适用时值为 `undefined`，而非从对象中省略。调用方可安全使用 `chunk.pageRange ?? null` 模式。

---

## `kb.status(): Promise<KBStatus>`

返回当前索引进度与健康状态，供宿主展示进度条或诊断问题。

```typescript
export interface KBStatus {
  // 文档级统计
  total: number            // documents 总数
  pendingParse: number
  parsing: number
  parsed: number           // BM25 已可搜
  indexed: number          // BM25 + 向量均已完成
  parseFailed: number

  // 向量覆盖率
  vectorCoverage: number   // embed_status=done / (done+pending+failed)
  chunkTotal: number
  chunkEmbedDone: number
  chunkEmbedPending: number
  chunkEmbedFailed: number

  // 健康检查
  walEnabled: boolean      // SQLite journal_mode=WAL
  writerLockHeld: boolean  // 当前实例是否持有写锁
  modelReady: boolean      // embedding 模型文件可用

  // 告警项（空数组表示健康）
  warnings: StatusWarning[]
}

export interface StatusWarning {
  type:
    | 'parse_failed'      // 有解析失败文档
    | 'missing_meta'      // 旧文档缺 blocks/charOffset，需重索引
    | 'model_not_found'   // 模型文件缺失
    | 'wal_disabled'      // WAL 未生效（异常）
  message: string
  docIds?: number[]       // 受影响的 docId 列表（parse_failed / missing_meta）
}
```

---

## `kb.reindexEmbeddings(model: EmbeddingModelSpec): Promise<void>`

重建向量库。清空 `chunks_vec`，重置所有 chunk 的 `embed_status=0`，更新 `model_tag`，触发 embedding 队列重新处理所有 chunk。

**重建期间**：`search()` 正常响应，以 BM25 + 存量向量（逐步增长）降级，`degraded` 字段说明「向量库重建中」。

**Throws**: `Error` — 若指定模型文件不存在（应先确保 `modelsDir` 中有对应 ONNX 文件）

---

## `kb.close(): Promise<void>`

优雅关闭：等待当前 batch 完成（不等待全部队列），提交 tantivy 写入，关闭 SQLite 连接，释放 flock 锁。多次调用幂等。

---

## 错误类型

```typescript
export class KBLockError extends Error {
  heldBy?: string   // 来自 .writer.lock 的人类可读诊断信息
}

export class KBModelMismatchError extends Error {
  expected: string  // 当前配置的 model_tag
  found: string     // 数据库中记录的 model_tag
}

export class ModelNotFoundError extends Error {
  modelsDir: string   // 查找模型的目录路径
  modelName: string   // 期望找到的模型名
}
```

---

## 远程解析服务契约

kb-core 向兼容以下契约的解析服务发送请求：

```
POST {parse.endpoint}/v1/parse
Content-Type: multipart/form-data

file: <文件二进制>
options: { "textLayerThreshold": 0.8 }

→ 200 OK
{
  "okf": {
    "parsedBy": "remote",
    "blocks": [
      {
        "blockId": 0,
        "type": "heading",
        "text": "...",
        "page": 1,
        "bbox": [10, 20, 200, 30],
        "fromImage": false
      }
    ]
  }
}

→ 4xx  不计入熔断失败次数
→ 5xx  计入熔断失败次数
→ 超时 计入熔断失败次数
```
