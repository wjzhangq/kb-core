# @wjzhangq/kb-core

[![npm](https://img.shields.io/npm/v/@wjzhangq/kb-core.svg)](https://www.npmjs.com/package/@wjzhangq/kb-core)

本地优先的知识库召回引擎：BM25 全文检索 + multilingual-e5-small 向量搜索 + RRF 融合排名。
纯 Node.js 原生插件（napi-rs），搜索时零 LLM 调用、零网络请求。

- 主包：[@wjzhangq/kb-core](https://www.npmjs.com/package/@wjzhangq/kb-core)
- 平台包（作为 optionalDependencies 自动安装）：
  - [@wjzhangq/kb-core-native-linux-x64-gnu](https://www.npmjs.com/package/@wjzhangq/kb-core-native-linux-x64-gnu)
  - [@wjzhangq/kb-core-native-win32-x64-msvc](https://www.npmjs.com/package/@wjzhangq/kb-core-native-win32-x64-msvc)

## 安装

```bash
npm install @wjzhangq/kb-core
# postinstall 自动下载 multilingual-e5-small ONNX 模型（约 60 MB）到 models/ 目录
```

跳过自动下载（CI / 离线环境）：

```bash
KB_SKIP_MODEL_DOWNLOAD=1 npm install
```

手动放置模型文件，或通过 `modelsDir` 配置项指向已有目录：

```
models/multilingual-e5-small/
  model_quantized.onnx
  tokenizer.json
  tokenizer_config.json
  special_tokens_map.json
```

## 快速开始

```js
const { KnowledgeBase } = require('@wjzhangq/kb-core')
const path = require('path')

const kb = new KnowledgeBase({
  dataDir: path.join(process.env.HOME, '.my-app/kb'),
  inference: { mode: 'local-first' },
  system: { maxCpuThreads: 2 },
})

// 添加文档（立即返回，后台异步建索引）
const added = await kb.add(['/path/to/notes.md', '/path/to/report.pdf'])
// => [{ docId: 1, path: '...', status: 'pending_parse' }, ...]

// 轮询直到 BM25 就绪（向量嵌入可继续在后台完成）
let s
do {
  await new Promise(r => setTimeout(r, 500))
  s = await kb.status()
} while (s.indexed === 0 && s.parsed === 0)

// 混合搜索（BM25 + 向量，RRF 融合）
const res = await kb.search('MQTT protocol IoT', { topN: 5 })
console.log(res.mode)                           // 'bm25+vec' | 'bm25-only'
console.log(res.results[0].chunks[0].matchedBy) // ['bm25', 'vector']

await kb.close()
```

## API

### `new KnowledgeBase(options)`

同步构造，在 `dataDir` 处打开（或创建）知识库，并启动后台解析和嵌入 worker。

**抛出**：
- `KBLockError` — 另一个写入进程已打开该数据库
- `KBModelMismatchError` — 数据库创建时的嵌入模型与当前配置不匹配

---

### `kb.add(path)`

```ts
kb.add(path: string | string[]): Promise<AddResult[]>
```

将文件路径加入索引队列，立即返回。同一路径重复 add 是幂等操作。

后台流水线：解析 → BM25 索引 → 向量嵌入。

支持的文件类型：`.md` `.txt` `.rst` `.pdf` `.docx` `.pptx` `.png` `.jpg` `.jpeg` `.eml` 及其他文本格式。

**返回值**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `docId` | `number` | 数据库内部 ID |
| `path` | `string` | 原始文件路径 |
| `status` | `string` | `'pending_parse'`（新加入）或 `'already_indexed'`（已存在） |

---

### `kb.search(query, options?)`

```ts
kb.search(query: string, options?: SearchOptions): Promise<SearchResponse>
```

BM25 + 向量混合搜索，RRF 融合排名。全部本地执行，零网络请求。

**SearchOptions**：

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `topK` | `number` | `50` | BM25 / 向量各自召回的候选文档数 |
| `topN` | `number` | `5` | RRF 融合后最终返回的文档数 |
| `rrfK` | `number` | `60` | RRF 平滑参数，值越大排名越分散 |
| `aggregate` | `'max' \| 'sum' \| 'top2sum'` | `'max'` | 同一文档多 chunk 得分的聚合方式 |
| `filter.docType` | `string[]` | — | 按文档类型过滤，可用值：`pdf` `text` `docx` `pptx` `image` `email` |
| `filter.paths` | `string[]` | — | 按路径前缀过滤 |
| `syntax` | `'text' \| 'fielded' \| 'raw'` | `'text'` | 查询语法。`raw` 为 tantivy 原生查询语法 |
| `maxCharsPerChunk` | `number` | `800` | 返回文本片段的最大字符数（超出时 `truncated=true`） |
| `includeText` | `boolean` | `true` | 是否在结果中包含原文 |
| `requireVector` | `boolean` | `false` | 为 `true` 时只返回有向量嵌入的结果 |

**SearchResponse 结构**：

```ts
interface SearchResponse {
  results:        SearchResult[]
  mode:           'bm25+vec' | 'bm25-only'
  vectorCoverage: number                     // 已嵌入 chunk 占比，0–1
  degraded:       { reason: string } | null  // 非 null 表示降级到纯 BM25
  timing:         SearchTiming
}

interface SearchResult {
  docId:  number
  path:   string
  title:  string | null
  score:  number           // RRF 聚合得分
  chunks: ChunkResult[]
}

interface ChunkResult {
  chunkId:     number
  text:        string          // 匹配片段原文（includeText=false 时为空）
  truncated:   boolean
  charOffset:  [number, number] // 片段在原文中的字节偏移 [start, end]
  pageRange:   [number, number] | null  // PDF 页码范围（1-indexed）
  bbox:        Bbox[] | null
  blockTypes:  string[]        // 如 ['text'], ['table'], ['image']
  fromImage:   boolean
  matchedBy:   ('bm25' | 'vector')[]
  score:       number
}

interface Bbox {
  page: number
  rect: [number, number, number, number]  // [x0, y0, x1, y1]，点单位
}

interface SearchTiming {
  parseMs: number; bm25Ms: number; embedMs: number
  vecMs: number;   rrfMs: number;  aggregateMs: number; totalMs: number
}
```

---

### `kb.status()`

```ts
kb.status(): Promise<KBStatus>
```

返回索引进度快照，可用于轮询直到搜索可用。

```ts
interface KBStatus {
  total: number; pendingParse: number; parsing: number
  parsed: number; indexed: number; parseFailed: number

  chunkTotal: number; chunkEmbedDone: number
  chunkEmbedPending: number; chunkEmbedFailed: number
  vectorCoverage: number   // 0–1

  walEnabled:     boolean
  writerLockHeld: boolean
  modelReady:     boolean
  warnings:       StatusWarning[]
}

interface StatusWarning {
  type:    'parse_failed' | 'missing_meta' | 'model_not_found' | 'wal_disabled'
  message: string
  docIds:  number[] | null
}
```

文档状态流转：`pending_parse` → `parsing` → `parsed` → `indexed`（或 `parse_failed`）。  
BM25 搜索在 `parsed` 阶段即可用；向量搜索需等到 `indexed` 且 `vectorCoverage > 0`。

---

### `kb.reindexEmbeddings(model)`

```ts
kb.reindexEmbeddings(model: EmbeddingModelSpec): Promise<void>
```

清除所有向量数据，用新模型重新嵌入。切换嵌入模型时使用。重建期间 BM25 搜索仍正常可用。

```ts
interface EmbeddingModelSpec {
  name:          string   // 模型目录名，如 'multilingual-e5-small'
  dim:           number   // 向量维度，如 384
  quantization?: string   // 默认 'int8'
}
```

---

### `kb.close()`

```ts
kb.close(): Promise<void>
```

等待 worker 退出，提交 tantivy 写入，释放 SQLite 写锁。幂等，多次调用安全。

**Windows 上必须调用**，否则持有的文件句柄会阻止数据目录的删除。

---

## 完整配置参数

```ts
new KnowledgeBase({
  dataDir: string,         // 必填，存储路径

  inference?: {
    mode: 'local-first' | 'bm25-only' | 'remote',  // 默认 'local-first'
    modelsDir?: string,          // ONNX 模型目录，默认 <package>/models/
    embedEndpoint?: string,      // mode='remote' 时必填
    model?: {
      name: string,              // 默认 'multilingual-e5-small'
      dim: number,               // 默认 384
      quantization?: string,     // 默认 'int8'
    },
    parse?: {                    // 远程 PDF 解析服务（可选）
      endpoint: string,
      allowRemote?: boolean,             // 默认 true
      textLayerThreshold?: number,       // 0–1，默认 0.8（文字层覆盖率低于此值则走远程解析）
      onRemoteParseUnavailable?: 'wait' | 'text-only' | 'skip',  // 默认 'wait'
      timeoutMs?: number,                // 默认 30000
      headers?: Record<string, string>,
      breaker?: {
        failureThreshold?: number,       // 默认 5
        resetTimeoutMs?: number,         // 默认 30000
      },
    },
  },

  system?: {
    maxCpuThreads?: number,     // 默认 2（tokio + rayon + ONNX 共享此预算）
    lowThreadPriority?: boolean, // 默认 true
    tempSecurity?: 'secure-temp' | 'acl-restricted',  // 默认 'secure-temp'
  },

  processing?: {
    chunkMaxTokens?: number,           // 默认 320
    chunkOverlapSentences?: number,    // 默认 2
    embedBatchSize?: number,           // 默认 16
    parseConcurrency?: number,         // 默认 4
    readerReloadIntervalMs?: number,   // 默认 5000
    maxFileSizeBytes?: number,         // 默认 104857600 (100 MB)
    attachmentDenyList?: string[],     // 默认 ['.exe','.dll','.bat','.sh','.app','.zip']
  },
})
```

## 模式选择

| 场景 | 推荐配置 |
|------|----------|
| 离线 / 隔离网络 | `mode: 'local-first'`（默认） |
| 只需全文检索，不需要向量 | `mode: 'bm25-only'`，无需下载模型 |
| 自托管嵌入服务 | `mode: 'remote'` + `embedEndpoint` |
| PDF / 图片 OCR 解析 | 配置 `inference.parse.endpoint` |

## 错误处理

```js
const { KnowledgeBase, KBLockErrorClass, KBModelMismatchErrorClass } = require('@wjzhangq/kb-core')

try {
  const kb = new KnowledgeBase({ dataDir: '/data/kb' })
} catch (err) {
  if (err instanceof KBLockErrorClass) {
    // 另一个进程已打开该数据库
  } else if (err instanceof KBModelMismatchErrorClass) {
    // 嵌入模型变更，需先调用 kb.reindexEmbeddings() 或删除数据目录重建
  }
}
```

## 离线模型管理

```js
// 方式一：环境变量（作用于 postinstall 和 KnowledgeBase 构造）
process.env.KB_MODELS_DIR = '/data/shared-models'

// 方式二：构造时传入
const kb = new KnowledgeBase({
  dataDir: '/data/kb',
  inference: { mode: 'local-first', modelsDir: '/data/shared-models' },
})
```

## 从源码编译

需要 Rust 工具链和 Node.js ≥ 18：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/wjzhangq/kb-core.git && cd kb-core
npm install
npm run build           # 生产构建（--release）
npm run build:debug     # 开发构建（不优化，编译快）
```

编译产物为项目根目录下的 `kb-core.<platform>-<arch>.node`，可直接 `require`。

> Windows 需安装 [MSVC 构建工具](https://visualstudio.microsoft.com/visual-cpp-build-tools/)；Linux 需要 `gcc`/`g++` 和 `pkg-config`。

## 发布流程

推送 `v*` tag 触发 `release.yml`：

1. Clippy + cargo test
2. 各平台编译（Linux x64、Windows x64）
3. tag 版本号写入 `package.json`
4. `napi pre-publish` 同步 `optionalDependencies` 版本
5. `npm pack` 打出 `.tgz`，上传到 GitHub Release

然后手动触发 `publish-npm.yml`，输入 tag 名，从 GitHub Release 下载 `.tgz` 并推送到 npm。  
README 随主包一起上传，npm 页面会自动展示。

## 许可证

MIT
