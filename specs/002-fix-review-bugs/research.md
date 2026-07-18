# Research: Fix High/Medium Severity Bugs — kb-core

**Date**: 2026-07-18 | **Branch**: `002-fix-review-bugs`

---

## 1. UTF-8 字符边界安全切片（FR-001）

### 现状

`chunk_text` 在 `parse.rs:227-258` 中以字节偏移（`start`、`end`）推进窗口：

```rust
let max_chars = cfg.chunk_max_tokens * 4;  // 实际是字节数，不是字符数
let end = (start + max_chars).min(text.len());
// text[start..end] 可能落在多字节字符中间 → panic
```

`text.len()` 返回字节长度，而中文字符占 3 字节。`rfind` 在 `text[start..end]` 上的调用同样要求边界对齐，否则 panic。

### 决策：使用 `floor_char_boundary`

**Decision**: 将字节上限对齐到合法字符边界，使用标准库 `str::floor_char_boundary()`（Rust 1.65 稳定，项目 rust-version 1.78 满足）。

同时改变步进单位：用字符数（`char_count`）而非字节数估算 max。实际上最简方案是保持字节步进，但每次截断前调一次 `floor_char_boundary` 对齐边界。

```rust
// 修复后伪码
let end_byte = (start + max_chars).min(text.len());
let end = text.floor_char_boundary(end_byte); // 保证合法
let slice = &text[start..end];               // 安全
```

断点优先级不变：`\n\n` > `\n` > ` `；`rfind` 也在安全 slice 上操作，无额外风险。

**Rationale**: `floor_char_boundary` 无额外依赖，一行修改，不改变已有的断点逻辑。

**Alternatives considered**:
- 改用 `char_indices()` 逐字符计数：正确但慢（O(n) 遍历），且改动量更大
- 改用 `unicode_segmentation` crate：正确且按词边界切，但新增依赖，超出最小改动范围

---

## 2. lin_blocks 安全映射（FR-002）

### 现状

```rust
fn build_linear_text(blocks: &[OkfBlock]) -> (String, Vec<(i64, i64)>) {
    for (i, block) in blocks.iter().enumerate() {
        // spans[i] 按 enumerate 顺序填充
    }
}
// 调用处：
lin_blocks[b.block_id as usize]  // block_id 可能 ≠ i
```

`build_linear_text` 用 `enumerate` 的 `i` 作为 Vec 下标填充，返回的 Vec 长度 = `blocks.len()`。但调用方用 `b.block_id as usize` 索引，如果 block_id 不是从 0 开始的连续整数（远程解析服务返回 1-based ID、或有间隙），必然越界。

### 决策：改用 HashMap<u32, (i64, i64)>

**Decision**: 修改 `build_linear_text` 返回类型为 `HashMap<u32, (i64, i64)>`，key 为 block_id；调用处改为 `lin_blocks.get(&b.block_id)` 并安全处理缺失（skip + warn）。

```rust
fn build_linear_text(blocks: &[OkfBlock]) -> (String, HashMap<u32, (i64, i64)>) {
    let mut spans: HashMap<u32, (i64, i64)> = HashMap::new();
    for block in blocks {
        let start = result.len() as i64;
        // ... append text
        spans.insert(block.block_id, (start, result.len() as i64));
    }
}
// 调用处：
if let Some(&(lin_start, lin_end)) = lin_blocks.get(&b.block_id) {
    // insert into DB
} else {
    tracing::warn!("block_id {} not in lin_blocks, skipping", b.block_id);
}
```

**Rationale**: HashMap 查找 O(1)，且对任意 block_id 命名空间都安全。`HashMap` 来自 `std::collections`，无新依赖。

**Alternatives considered**:
- 改 `build_linear_text` 用 block_id 作 Vec 下标（要求 ID 连续）：不安全
- 保留 Vec，传 block 引用进去自建映射：逻辑等价但代码更复杂

---

## 3. parse 失败状态落库（FR-003）

### 现状深挖

重读代码后发现情况比初始报告复杂：

- `process_doc` 在 `try_parse` 失败时 **已有** `parse_failed` 更新（`parse.rs:76-83`）
- 但该 DB update 本身用了 `?`——若 DB 操作失败，error propagates 给 spawner，spawner 仅打日志，状态停在 `parsing`
- 更隐蔽的场景：`process_doc` 在初始 DB 查询（取 path/doc_type）时失败，此时状态未变为 `parsing`，停在 `pending_parse`
- 正确结论：`parse_failed` 更新本身存在，但不是幂等/兜底的——spawner 应该**无论如何**在 `process_doc` 返回 `Err` 时做一次 best-effort 的状态更新

### 决策：spawner 补兜底更新

**Decision**: 在 spawner 的 `Err` 分支（`parse.rs:35-39`）补充：拿一次 db 锁做 best-effort `parse_failed` 更新，失败则再次打日志，不 panic：

```rust
if let Err(e) = process_doc(...).await {
    tracing::error!("parse failed for doc {}: {:#}", doc_id, e);
    // Best-effort fallback: ensure status is not left as 'parsing'
    let guard = db.lock().await;
    let _ = guard.conn.execute(
        "UPDATE documents SET status='parse_failed', updated_at=?1 WHERE doc_id=?2 AND status='parsing'",
        params![now_ms(), doc_id],
    );
}
```

用 `AND status='parsing'` 避免覆盖 `process_doc` 已正确写入的 `parse_failed`（带详细 error 字段）。

**Rationale**: 双重保障：`process_doc` 内部先写（有完整 error 信息），spawner 再兜底（简单确保状态不卡死）。

---

## 4. BM25 查询解析失败返回空结果（FR-004）

### 现状

`bm25.rs:30-34`：
```rust
let query = query_parser.parse_query(query_str)
    .unwrap_or_else(|_| Box::new(tantivy::query::AllQuery));
```

`AllQuery` 匹配索引内所有文档，相当于无条件全表扫描。

### 决策：返回 `Ok(vec![])`

**Decision**: 解析失败时提前返回空结果，不执行搜索：

```rust
let query = match query_parser.parse_query(query_str) {
    Ok(q) => q,
    Err(_) => return Ok(vec![]),
};
```

**Rationale**: 格式错误的查询字符串没有语义，返回空结果符合用户期望（"没找到"），不污染搜索结果，也避免无意义的全库扫描。

**Alternatives considered**:
- 返回 `Err`：会导致上层搜索抛异常，影响 Node.js 调用方
- 记录 warn + 返回空：可以加，但不是 spec 要求的，范围外

---

## 5. IndexReader 单例化（FR-005）

### 现状

`TantivyIndex` struct 无 reader 字段，每次 `bm25::search` 调用 `tantivy.reader()?` 都在 `IndexReader::builder().try_into()` 里创建新实例，每个实例持有后台 segment-reload 线程。

### 决策：存入 TantivyIndex struct

**Decision**: 在 `TantivyIndex` 中增加 `reader: IndexReader` 字段，在 `open_or_create` 时创建一次：

```rust
pub struct TantivyIndex {
    pub index: Index,
    pub schema: TantivySchema,
    writer: Mutex<Option<IndexWriter>>,
    reader: IndexReader,     // 新增
}

// open_or_create 末尾：
let reader = index.reader_builder()
    .reload_policy(ReloadPolicy::Manual)
    .try_into()?;
```

`bm25::search` 改为接收 `&IndexReader` 或直接调用 `tantivy.reader()` 改为 `&tantivy.reader`（pub field 或 getter 方法）。

**Rationale**: `IndexReader` 设计即为复用（tantivy 文档推荐模式），`Manual` reload 策略下由调用方显式控制可见性。

---

## 6. embed 失败更新 doc status（FR-006）

### 现状

`embed.rs:62-72`：embed batch 失败时仅将 chunks 标为 `embed_status=2`，`documents.status` 停在 `parsed`。用户无法通过 `status()` 知道向量嵌入失败了。

### 决策：更新受影响文档的 status 为 `embed_failed`

**Decision**: 在标记 chunks 失败后，额外更新这批 chunks 所属文档的 status：

```rust
// 现有：标记 chunks
guard.conn.execute("UPDATE chunks SET embed_status=2 WHERE chunk_id=?1", ...)?;
// 新增：标记文档
guard.conn.execute(
    "UPDATE documents SET status='embed_failed', updated_at=?1
     WHERE doc_id=(SELECT doc_id FROM chunks WHERE chunk_id=?2)",
    params![now_ms(), chunk_id],
)?;
```

同步更新 `index.d.ts` 中 `status` 字段的 union type 加入 `'embed_failed'`。

---

## 7. 消除持锁 await 死锁风险（FR-007）

### 现状

`lib.rs:200-202`：
```rust
let guard = db.lock().await;     // tokio::sync::Mutex 锁
// ... DB insert ...
let _ = self.parse_tx.send(doc_id).await;  // 持锁期间 await channel send
```

若 `parse_tx` channel 已满，`send` 无限等待，同时 parse worker 也需要 db 锁才能标记 parsing 状态 → 死锁。

### 决策：锁内取值，锁外 await

**Decision**: 提取锁外需要的值，释放锁再 await：

```rust
let doc_id_opt = {
    let guard = db.lock().await;
    // ... DB insert ...
    if changes > 0 { Some(guard.conn.last_insert_rowid()) } else { None }
};  // guard dropped here

if let Some(doc_id) = doc_id_opt {
    let _ = self.parse_tx.send(doc_id).await;
    // ...
}
```

**Rationale**: 标准 Tokio 模式：持有 `tokio::sync::Mutex` 不得跨 `.await`，否则会阻塞运行时线程。

---

## Summary

| FR | 修复方案 | 改动文件 | 新依赖 |
|----|---------|---------|-------|
| FR-001 | `floor_char_boundary` 对齐字节边界 | `parse.rs` | 无 |
| FR-002 | Vec → `HashMap<u32, (i64,i64)>` | `parse.rs` | 无 |
| FR-003 | spawner 补 best-effort 兜底更新 | `parse.rs` | 无 |
| FR-004 | 解析失败 early return `Ok(vec![])` | `bm25.rs` | 无 |
| FR-005 | `IndexReader` 存入 struct，单次创建 | `tantivy_idx/mod.rs`, `bm25.rs` | 无 |
| FR-006 | embed 失败同步更新 doc status | `embed.rs`, `index.d.ts` | 无 |
| FR-007 | 锁外 await channel send | `lib.rs` | 无 |
