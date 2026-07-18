# Quickstart Validation Guide: kb-core 召回引擎

## 前置条件

| 条件 | 说明 |
|------|------|
| Rust 1.78+ | `rustup show` 确认 |
| Node.js 18+ | `node --version` |
| `@napi-rs/cli` 3.x | `npm install -g @napi-rs/cli` |
| 网络访问（首次） | postinstall 下载 multilingual-e5-small（~60MB） |

---

## 1. 安装与构建

```bash
# 安装依赖（触发 postinstall 下载模型）
npm install

# 构建 Rust native addon（开发模式）
npm run build:debug
# 产物：kb-core.darwin-arm64.node（或当前平台对应文件）
```

验证模型已下载：
```bash
ls models/multilingual-e5-small/
# 期望：model_quantized.onnx  tokenizer.json  tokenizer_config.json
```

---

## 2. 基础验证：构造与关闭

```javascript
const { KnowledgeBase } = require('./index.js')
const os = require('os')
const path = require('path')

const kb = new KnowledgeBase({
  dataDir: path.join(os.tmpdir(), 'kb-test'),
  inference: { mode: 'local-first' },
  system: { maxCpuThreads: 2 }
})

// 期望：无异常，.writer.lock 文件存在于 dataDir
await kb.close()
// 期望：无异常，flock 锁释放
```

---

## 3. 异步索引 + BM25 先行验证（User Story 1）

```javascript
const results = await kb.add([
  '/path/to/docs/intro.md',
  '/path/to/docs/mqtt-guide.md'
])
// 期望：立即返回 [{docId: 1, status: 'pending_parse'}, ...]

// 轮询直到 BM25 可搜（解析完成，无需等 embedding）
await waitFor(async () => {
  const s = await kb.status()
  return s.parsed + s.indexed >= 2
}, { timeoutMs: 30_000 })

// 此时 BM25 已可命中，向量可能还未就绪
const r = await kb.search('MQTT protocol', { topN: 3 })
// 期望：results 不为空，mode 为 'bm25-only' 或 'bm25+vec'（取决于 embedding 进度）
// 期望：r.results[0].chunks[0].matchedBy 包含 'bm25'
```

---

## 4. 混合检索验证（User Story 2）

```javascript
// 等待 embedding 追平
await waitFor(async () => {
  const s = await kb.status()
  return s.vectorCoverage >= 0.99
}, { timeoutMs: 120_000 })

const r = await kb.search('BLE advertising packet', { topN: 5 })

// 验证
assert(r.mode === 'bm25+vec')
assert(r.vectorCoverage > 0.99)
assert(r.results.length > 0)
const chunk = r.results[0].chunks[0]
assert(Array.isArray(chunk.matchedBy))           // ['bm25'] 或 ['vector'] 或 ['bm25','vector']
assert(typeof chunk.charOffset[0] === 'number')  // charOffset 有效
assert(Array.isArray(chunk.blockTypes))          // blockTypes 有效
```

---

## 5. 来源 meta 完整性验证（FR-010）

```javascript
// 用含分页 PDF 的文档（需远程解析服务配置）
// 或用纯文本文档验证 blockTypes/charOffset
const r = await kb.search('introduction', { topN: 1 })
const chunk = r.results[0].chunks[0]

assert(chunk.charOffset[1] > chunk.charOffset[0])  // charOffset 非零区间
assert(chunk.blockTypes.length > 0)                // 至少一个块类型
// fromImage: false（纯文本文档）
assert(chunk.fromImage === false)
```

---

## 6. 故障隔离验证（FR-015）

```javascript
// 添加一个不存在的文件（触发解析失败）
await kb.add('/nonexistent/file.pdf')

await waitFor(async () => {
  const s = await kb.status()
  return s.warnings.some(w => w.type === 'parse_failed')
}, { timeoutMs: 10_000 })

const s = await kb.status()
const warn = s.warnings.find(w => w.type === 'parse_failed')
assert(warn !== undefined)
assert(warn.docIds.length > 0)

// 其他文档仍可正常搜索（宿主进程未崩溃）
const r = await kb.search('test query')
assert(r !== null)  // 无异常
```

---

## 7. 模型迁移验证（User Story 4）

```javascript
// 构造时指定旧模型 tag（模拟升级场景）
// 注：需先用 bge-small-zh 建库，再换 multilingual-e5-small 打开
try {
  const kb2 = new KnowledgeBase({
    dataDir: '/path/to/old-kb',
    inference: { mode: 'local-first' }  // 默认 multilingual-e5-small
  })
} catch (e) {
  assert(e.constructor.name === 'KBModelMismatchError')
  assert(typeof e.expected === 'string')
  assert(typeof e.found === 'string')
}
```

---

## 8. 零出站验证（SC-004）

```bash
# 使用 macOS 网络监控工具，或在 Docker 中启动并断网
# 仅索引 .md/.txt/.eml 纯文本文档，不配置 parse.endpoint

# 运行索引
node -e "
const {KnowledgeBase} = require('./index.js')
const kb = new KnowledgeBase({ dataDir: '/tmp/kb-offline', inference: { mode: 'local-first' } })
kb.add(['README.md']).then(() => setTimeout(() => kb.close(), 5000))
"

# 期望：全程无出站 TCP 连接（除本机 localhost）
```

---

## 9. CI 验证（GitHub Actions）

```bash
# 推送 PR 触发 ci.yml
# 期望：cargo test 通过，vitest 通过，clippy 无 error

# 推送 tag 触发 publish.yml
git tag v0.1.0
git push origin v0.1.0
# 期望：5 个平台 .node 产物上传到 Release，npm 包发布成功
```

---

## 辅助函数

```javascript
async function waitFor(predFn, { timeoutMs = 30_000, intervalMs = 500 } = {}) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await predFn()) return
    await new Promise(r => setTimeout(r, intervalMs))
  }
  throw new Error('waitFor timed out')
}
```

---

## 相关文档

- API 契约详情：[contracts/node-api.md](contracts/node-api.md)
- 数据模型：[data-model.md](data-model.md)
- 技术决策：[research.md](research.md)
