# Quickstart Validation Guide: Fix High/Medium Severity Bugs

**Date**: 2026-07-18 | **Branch**: `002-fix-review-bugs`

本文档说明如何验证7项修复的正确性。不包含实现代码，只描述可运行的验证命令和预期结果。

---

## Prerequisites

```bash
# 在 repo root
cargo build   # 确保编译通过
npm run build:debug   # 构建 napi addon
```

---

## Scenario 1 — CJK 文档不 panic（FR-001）

**验证方式**: Rust 单元测试

```bash
cargo test test_chunk_cjk -- --nocapture
```

**测试用例要点**（写在 `tests/rust/test_pipeline.rs`）:
- 输入：纯中文字符串，长度使默认切块边界落在汉字中间
- 期望：返回非空 chunk 列表，每个 `chunk.text` 为合法 UTF-8，不 panic
- 边界用例：单个 4 字节 emoji 位于切块边界处

**预期输出**: 测试 PASS，无 `panic: byte index X is not a char boundary`

---

## Scenario 2 — 含空段落文档不 panic（FR-002）

**验证方式**: Rust 单元测试

```bash
cargo test test_empty_paragraph -- --nocapture
```

**测试用例要点**:
- 构造一个 `OkfBlock` 列表，其中第一个 block 的 `block_type` 为空（空段落），`block_id` 不从 0 开始
- 调用 `build_linear_text` + 写库流程
- 期望：成功完成，空段落被 skip 并打 warn 日志，无 panic

**预期输出**: 测试 PASS，无 `index out of bounds`

---

## Scenario 3 — parse 失败状态落库（FR-003）

**验证方式**: Node.js 集成测试

```bash
KB_SKIP_MODEL_DOWNLOAD=1 npm test -- --reporter=verbose -t "parse_failed"
```

**测试用例要点**（写在 `tests/node/status.test.ts`）:
- 添加一个不存在的文件路径（必然解析失败）
- 等待 2000ms 后调用 `kb.status()`
- 期望：该文档的 status 为 `parse_failed`，不为 `parsing`

**预期输出**: 测试 PASS；status 响应中目标文档 `status === 'parse_failed'`

---

## Scenario 4 — 格式错误查询返回空结果（FR-004）

**验证方式**: Rust 单元测试

```bash
cargo test test_bm25_invalid_query -- --nocapture
```

**测试用例要点**（写在 `tests/rust/test_search.rs`）:
- 索引若干文档
- 用 `"[unclosed bracket"` 等格式非法查询调用 BM25 搜索
- 期望：返回 `Ok(vec![])`，结果数为 0

**预期输出**: 测试 PASS；结果为空，非全库文档数

---

## Scenario 5 — IndexReader 不泄漏（FR-005）

**验证方式**: 长跑压测（手动）

```bash
node -e "
const { KnowledgeBase } = require('.')
const kb = new KnowledgeBase({ dataDir: '/tmp/kb-leak-test', inference: { mode: 'bm25-only' } })
;(async () => {
  await kb.add(['/path/to/any/file.txt'])
  await new Promise(r => setTimeout(r, 500))
  for (let i = 0; i < 1000; i++) await kb.search('test')
  const before = process.memoryUsage().rss
  for (let i = 0; i < 1000; i++) await kb.search('test')
  const after = process.memoryUsage().rss
  console.log('RSS delta:', (after - before) / 1024, 'KB (expect < 500KB)')
  await kb.close()
})()
"
```

**预期输出**: RSS delta < 500KB，文件句柄数（可用 `lsof -p <pid>` 观测）不随请求数增长

---

## Scenario 6 — embed 失败状态可见（FR-006）

**验证方式**: Node.js 集成测试（需要能够注入 embed 失败的测试环境）

```bash
KB_SKIP_MODEL_DOWNLOAD=1 npm test -- -t "embed_failed"
```

**测试用例要点**（写在 `tests/node/status.test.ts`）:
- 使用 `mode: 'local-first'` 但故意提供不存在的模型路径，触发 embed 失败
- 添加文档，等待 BM25 索引完成后再等待 embed 失败
- 期望：BM25 搜索仍可返回结果；`status()` 中该文档 status 为 `embed_failed`

**预期输出**: 测试 PASS；BM25 搜索正常，status 为 `embed_failed`

---

## Scenario 7 — 无死锁（FR-007）

**验证方式**: 并发压测

```bash
node -e "
const { KnowledgeBase } = require('.')
const kb = new KnowledgeBase({ dataDir: '/tmp/kb-deadlock-test', inference: { mode: 'bm25-only' } })
;(async () => {
  // 并发 add + search，复现之前持锁 await 的条件
  const files = Array.from({ length: 20 }, (_, i) => '/tmp/test' + i + '.txt')
  files.forEach(f => require('fs').writeFileSync(f, 'test content ' + f))
  await Promise.all([
    ...files.map(f => kb.add([f])),
    ...Array.from({ length: 10 }, () => kb.search('test')),
  ])
  console.log('completed without deadlock')
  await kb.close()
})().catch(e => { console.error('DEADLOCK or ERROR:', e); process.exit(1) })
"
```

**预期输出**: `completed without deadlock`，进程正常退出（非挂起）

---

## 全量回归

```bash
# Rust 测试套件
cargo test

# Node 集成测试
KB_SKIP_MODEL_DOWNLOAD=1 npm test
```

**通过标准**: 全部测试 PASS，无新增 failure。详见 [spec.md SC-006](./spec.md)。
