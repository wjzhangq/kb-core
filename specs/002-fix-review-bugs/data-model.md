# Data Model: Fix High/Medium Severity Bugs — kb-core

**Date**: 2026-07-18 | **Branch**: `002-fix-review-bugs`

本次修复以缺陷修正为主，数据模型改动集中在**文档状态机的新增状态**和**内部数据结构的类型变更**，不涉及 schema 表结构变更（无 migration）。

---

## 1. Document 状态机

### 现有状态

| status | 含义 |
|--------|------|
| `pending_parse` | 已入队，等待解析 |
| `parsing` | 正在解析 |
| `parsed` | 解析完成，等待/正在嵌入 |
| `indexed` | 已完成 BM25 索引（向量可能仍在进行） |
| `parse_failed` | 解析失败（已存在，但落库不完整 → FR-003 修复） |

### 新增状态

| status | 含义 | 引入 FR |
|--------|------|---------|
| `embed_failed` | 向量嵌入失败（BM25 索引仍有效） | FR-006 |

### 状态转换（修复后）

```
pending_parse ──parse成功──▶ parsed ──embed成功──▶ indexed
      │                        │
      │parse失败                │embed失败
      ▼                        ▼
 parse_failed              embed_failed
```

**约束**:
- `parse_failed` MUST 在解析失败后可查询到（FR-003）；spawner 兜底更新仅在 `status='parsing'` 时生效，不覆盖 `process_doc` 已写入的详细错误
- `embed_failed` 不影响 BM25 可搜索性；该文档的 chunks 仍在 tantivy 索引中
- `embed_failed` 文档的 chunks `embed_status=2`，与文档级状态一致

---

## 2. Block → 线性区间映射（内部结构）

### 变更

| | 变更前 | 变更后 |
|--|--------|--------|
| `build_linear_text` 返回类型 | `(String, Vec<(i64, i64)>)` | `(String, HashMap<u32, (i64, i64)>)` |
| 索引方式 | `lin_blocks[block_id as usize]`（越界 panic） | `lin_blocks.get(&block_id)`（安全） |
| 缺失 block_id | panic | skip + warn |

**验证规则**:
- HashMap key 为 `OkfBlock.block_id`（u32）
- 缺失的 block_id 对应的 block 在写库时被跳过，不写入 `blocks` 表
- value `(lin_start, lin_end)` 为该 block 在线性文本中的字节区间

---

## 3. Chunk 边界（内部约束）

`ParsedChunk` 结构不变，但生成逻辑约束加强：

| 字段 | 约束（修复后） |
|------|--------------|
| `char_start` / `char_end` | MUST 落在合法 UTF-8 字符边界上（FR-001） |
| `text` | MUST 为完整字符序列，不含被截断的多字节字符 |

**不变量**: 对任意输入文本 `t`，`chunk_text(t)` 产生的所有 chunk 的 `text[start..end]` 切片操作均不 panic。

---

## 4. 无 schema 变更

- 不新增/修改 SQLite 表结构
- 不新增 migration 版本
- `embed_failed` / `parse_failed` 是 `documents.status` 列（TEXT）的新取值，非新列
