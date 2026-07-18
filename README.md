# kb-core

本地优先的知识库召回引擎：BM25 全文检索 + multilingual-e5-small 向量搜索 + RRF 融合排名。纯 Node.js 原生插件（napi-rs）。搜索时零 LLM 调用、零网络请求。

## 安装

```bash
npm install kb-core
# postinstall 会将 multilingual-e5-small ONNX 模型（约 60 MB）下载到 models/ 目录
```

如果下载失败，可手动放置模型文件：
```
models/multilingual-e5-small/
  model_quantized.onnx
  tokenizer.json
  tokenizer_config.json
  special_tokens_map.json
```
或在运行时指定自定义目录：
```js
new KnowledgeBase({ dataDir: '...', inference: { mode: 'local-first', modelsDir: '/path/to/models' } })
```

## 快速开始

```js
const { KnowledgeBase } = require('kb-core')
const path = require('path')

const kb = new KnowledgeBase({
  dataDir: path.join(process.env.HOME, '.my-app/kb'),
  inference: { mode: 'local-first' },
  system: { maxCpuThreads: 2 },
})

// 添加文档（立即返回，后台异步建索引）
const added = await kb.add(['/path/to/notes.md', '/path/to/report.pdf'])
// [{ docId: 1, path: '...', status: 'pending_parse' }, ...]

// 轮询直到 BM25 就绪（速度很快，无需等待向量完成）
let status
do {
  status = await kb.status()
  await new Promise(r => setTimeout(r, 500))
} while (status.parsed + status.indexed === 0)

// 搜索（BM25 + 向量，RRF 融合）
const results = await kb.search('MQTT protocol IoT', { topN: 5 })
console.log(results.mode)      // 'bm25+vec' 或 'bm25-only'
console.log(results.results[0].chunks[0].matchedBy)  // ['bm25', 'vector']

await kb.close()
```

## API

### `new KnowledgeBase(options: KBConfig)`

在 `options.dataDir` 处打开（或创建）知识库。同步操作。

**抛出异常**：
- `KBLockError` — 另一个写入进程已打开该数据库
- `KBModelMismatchError` — 数据库创建时使用的嵌入模型与当前不同

### `kb.add(path: string | string[]): Promise<AddResult[]>`

将文档加入索引队列，立即返回。后台流水线：解析 → BM25 索引 → 向量嵌入。

### `kb.search(query: string, options?: SearchOptions): Promise<SearchResponse>`

BM25 + 向量混合搜索，RRF 融合排名。全部本地执行，零网络请求。

### `kb.status(): Promise<KBStatus>`

返回索引进度、向量覆盖率及健康告警。

### `kb.reindexEmbeddings(model: EmbeddingModelSpec): Promise<void>`

使用新模型重建向量索引。重建期间搜索仍可用（仅 BM25 模式）。

### `kb.close(): Promise<void>`

刷新并关闭。幂等操作。

## KBConfig 参数说明

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dataDir` | `string` | — | 存储 `kb.db` 和 tantivy 索引的路径 |
| `inference.mode` | `'local-first' \| 'bm25-only' \| 'remote'` | `'local-first'` | 推理模式 |
| `inference.modelsDir` | `string` | `<package>/models/` | ONNX 模型文件路径 |
| `system.maxCpuThreads` | `number` | `2` | 线程预算（tokio + rayon + ONNX 内部并行） |
| `system.lowThreadPriority` | `boolean` | `true` | 降低后台线程优先级（不可逆） |
| `processing.chunkMaxTokens` | `number` | `320` | 每个分块的最大 token 数 |
| `processing.embedBatchSize` | `number` | `16` | 嵌入批处理大小 |
| `processing.parseConcurrency` | `number` | `4` | 并行解析 worker 数 |

## 模式选择矩阵

| 需求 | 推荐配置 |
|------|----------|
| 离线 / 隔离网络环境 | `mode: 'local-first'`（默认） |
| 不需要向量嵌入，优先速度 | `mode: 'bm25-only'` |
| 自托管嵌入服务 | `mode: 'remote'`，配合 `embedEndpoint` |
| PDF / 图片文档 | 配置 `inference.parse.endpoint` |

## CI 与发布

- `npm run build:debug` — 开发构建
- `npm run build` — 生产构建
- **仅** 推送 `v*` tag 触发编译与发布（off-tag 不做任何构建）
- 发布平台：`linux-x64-gnu`、`linux-arm64-gnu`、`win32-x64-msvc`
- ONNX Runtime 通过 `ort-download-binaries` 在构建时静态链接进 `.node`，发布产物自包含，用户机器无需单独的 onnxruntime 动态库

## `onModelDownloadRequired`

如果 `postinstall` 失败（CI 环境、离线场景），可以延迟模型下载：

```js
// 安装时设置环境变量跳过 postinstall：
KB_SKIP_MODEL_DOWNLOAD=1 npm install

// 然后在运行时、创建 KnowledgeBase 之前：
process.env.KB_MODELS_DIR = '/path/to/pre-downloaded/models'
```

## 许可证

MIT
