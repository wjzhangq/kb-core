# Feature Specification: Fix High/Medium Severity Bugs from Code Review

**Feature Branch**: `002-fix-review-bugs`

**Created**: 2026-07-18

**Status**: Draft

**Input**: 修复 kb-core 代码审查发现的 bug：parse.rs 字节切片 panic、lin_blocks 越界 panic、parse 失败状态不落库，以及其他 HIGH/MEDIUM 级别问题

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — 中文及多字节内容可正常索引 (Priority: P1)

用户将包含中文、日文或其他多字节字符的文档（`.md`、`.txt`）加入知识库，期望文档被正确解析、切块并建立索引，而不出现崩溃。

**Why this priority**: 当前 `chunk_text` 按字节偏移截断字符串，任何多字节字符出现在切块边界处都会引发 panic，中文文档必现，属于最高优先级阻塞缺陷。

**Independent Test**: 单独向知识库添加一个含中文内容的 Markdown 文件，等待解析完成后调用 `status()`，文档状态应变为 `indexed`，整个过程不抛出异常。

**Acceptance Scenarios**:

1. **Given** 知识库已初始化，**When** 添加一个全中文内容的 `.md` 文件，**Then** 文档被完整切块并进入 BM25 索引，`status()` 返回该文档已完成，进程不崩溃
2. **Given** 知识库已初始化，**When** 添加一个中英混排且中文字符恰好落在默认切块边界处的文件，**Then** 切块在合法字符边界处断开，内容完整，无 panic

---

### User Story 2 — 含空段落的文档可正常索引 (Priority: P1)

用户添加一个文档，该文档经解析后产生空段落（例如仅含空行的 Markdown 块），期望整个索引流程正常完成而不崩溃。

**Why this priority**: `lin_blocks[b.block_id as usize]` 以 block_id 作为 Vec 下标，但空段落在 filter 时被移除导致 block_id 与 Vec 长度不对应，遇到含空段落的文档必现 panic。

**Independent Test**: 添加一个开头含空行或空段落的文档，检查其最终索引状态为 `indexed`，进程不崩溃。

**Acceptance Scenarios**:

1. **Given** 知识库已初始化，**When** 添加一个首段为空白行的 Markdown 文件，**Then** 文档被正确索引，`status()` 不显示 `parsing` 卡死
2. **Given** 知识库已初始化，**When** 添加一个含多个连续空段落的文档，**Then** 非空内容被正确切块并索引，空段落被安全跳过

---

### User Story 3 — 解析失败的文档有明确的失败状态 (Priority: P1)

用户添加一批文档，其中某个文档因损坏或格式不支持而解析失败，期望该文档的状态被标记为失败，而其他文档的索引不受影响，且重启后不会因卡死文档而阻塞后续队列。

**Why this priority**: 解析失败时仅打印日志、不更新数据库状态，导致文档永久停留在 `parsing`，进程重启后还会被反复重试，阻塞后续文档。

**Independent Test**: 向知识库添加一个已知损坏的文件，等待解析超时或失败后调用 `status()`，该文档状态应为 `parse_failed`，其余文档不受影响。

**Acceptance Scenarios**:

1. **Given** 知识库已初始化，**When** 添加一个无效/损坏文件，**Then** 该文档的状态被更新为 `parse_failed`，不永久停留在 `parsing`
2. **Given** 一批文档中有一个解析失败，**When** 其他文档解析成功，**Then** 成功文档正常索引，失败文档不阻塞队列
3. **Given** 进程重启后数据库存在 `parse_failed` 文档，**When** 启动恢复逻辑，**Then** 不会将 `parse_failed` 文档重新加入解析队列

---

### User Story 4 — BM25 搜索对格式错误的查询有明确行为 (Priority: P2)

用户输入一个格式错误的查询字符串（如特殊符号、不平衡引号），期望搜索返回空结果或错误提示，而不是静默扫描整个知识库并返回不相关结果。

**Why this priority**: 查询解析失败时 fallback 到 `AllQuery`，会返回整库内容，从用户角度看是"搜什么都有结果"，语义误导且性能损耗大。

**Independent Test**: 用格式无效的查询字符串（如 `"[unclosed`）调用 `search()`，返回结果数量为 0 或附带解析失败提示，不等于总文档数。

**Acceptance Scenarios**:

1. **Given** 知识库含 100 条已索引文档，**When** 用格式无效的查询调用 search，**Then** 返回结果数量为 0，不等于全库文档数
2. **Given** 知识库含已索引文档，**When** 用空字符串调用 search，**Then** 返回空结果列表，不崩溃

---

### User Story 5 — 向量搜索组件的 IndexReader 不持续泄漏 (Priority: P2)

作为系统运维者，在知识库长时间运行并持续接收搜索请求时，期望进程内存和文件句柄占用保持稳定，不随搜索次数增长而无限增加。

**Why this priority**: 当前每次搜索都创建新的 `IndexReader`，该对象持有后台 segment-reload 线程和文件句柄，长期运行会导致资源泄漏。

**Independent Test**: 对一个已索引的知识库执行 1000 次搜索请求，用系统工具观测进程的文件句柄数和内存使用，两项指标不应随请求数线性增长。

**Acceptance Scenarios**:

1. **Given** 知识库运行中，**When** 执行大量搜索请求，**Then** 进程文件句柄数保持稳定（不超过初始值 + 固定上限）
2. **Given** 知识库实例关闭，**When** 调用 `close()`，**Then** 所有 IndexReader 关联资源正常释放

---

### User Story 6 — embed 失败的文档有明确的失败状态 (Priority: P2)

用户向知识库添加文档并触发向量嵌入，当嵌入批次整体失败时，期望该文档状态被标记为 `embed_failed`，而非永久停留在 `parsed`。

**Why this priority**: embed 失败时文档状态不更新，调用方无法感知失败，文档在用户看来处于"已解析但未索引"的僵死状态。

**Independent Test**: 使用损坏/不存在的模型路径触发嵌入失败，调用 `status()`，文档状态应为 `embed_failed`；BM25 搜索不受影响。

**Acceptance Scenarios**:

1. **Given** embed 批次整体失败，**When** 失败后查询 `status()`，**Then** 文档状态为 `embed_failed`，不停留在 `parsed`
2. **Given** embed 失败但 BM25 索引已完成，**When** 执行搜索，**Then** 搜索结果不受影响，仅向量召回降级

---

### User Story 7 — 并发 add() 不因持锁 await 死锁 (Priority: P2)

运维者在高并发场景下同时调用 `add()` 和 `search()`，期望系统不挂起、所有调用在有限时间内返回。

**Why this priority**: 当 parse channel 满时，`add()` 在持有 DB 锁的状态下 await channel send，导致持有锁的线程阻塞，进而死锁整个系统。

**Independent Test**: 并发执行 20 次 `add()` + 10 次 `search()`，所有调用在合理时间内完成，无死锁。

**Acceptance Scenarios**:

1. **Given** parse channel 已满，**When** 并发调用 add 和 search，**Then** 所有调用正常完成，进程不挂起
2. **Given** 正常负载下并发 add + search，**When** 系统运行，**Then** 响应时间与串行执行相近，无异常延迟

---

### Edge Cases

- 文档内容完全为空白字符（空格、换行）时，应产生零个 chunk 而非 panic
- block_id 为非连续整数（如远程解析服务返回的 ID 非从 0 起）时，不越界
- 解析超时（远程解析端点不可达）时，文档状态被标记为 `parse_failed` 而非永久 `parsing`
- embed 批次部分失败时，已成功的 chunk 保留 embed 结果，失败的 chunk 状态可被查询
- 多个进程/线程并发 close() 时，IndexReader 资源不重复释放

---

## Requirements *(mandatory)*

### Functional Requirements

**P1 — 解析引擎修复**

- **FR-001**: 系统 MUST 在切块时按 Unicode 字符边界（而非字节偏移）进行字符串分割，保证任意 UTF-8 文本不引发 panic
- **FR-002**: 系统 MUST 构建 block_id 到线性区间的映射时，不依赖 block_id 作为连续 Vec 下标；遇到不存在的 block_id 时安全跳过或返回默认值，不 panic
- **FR-003**: 系统 MUST 在文档解析失败（任何原因）时，将数据库中该文档的状态更新为 `parse_failed`，不得遗留在 `parsing` 状态

**P2 — 搜索健壮性**

- **FR-004**: 系统 MUST 在 BM25 查询解析失败时返回空结果集，MUST NOT 静默 fallback 到全库扫描
- **FR-005**: 系统 MUST 在初始化时创建单个 `IndexReader` 实例并在整个生命周期内复用，MUST NOT 在每次搜索时创建新实例

**P2 — 向量嵌入状态**

- **FR-006**: 系统 MUST 在 embed 批次整体失败时，将受影响文档的状态更新为可识别的错误状态，MUST NOT 让文档永久停留在 `parsed`
- **FR-007**: 系统 MUST 不在持有数据库锁期间执行异步等待操作，以消除 channel 满时的死锁风险

### Key Entities

- **Document**：知识库中的文档，具有 `status` 字段（`pending_parse` / `parsing` / `parsed` / `indexed` / `parse_failed` / `embed_failed`）
- **Chunk**：文档被切分后的文本块，具有字符级偏移 `char_start` / `char_end` 和 `embed_status`
- **Block**：解析器返回的结构化段落块，具有 `block_id` 及对应的线性文本区间

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 包含中文内容的文档成功完成索引的比例达到 100%（当前为 0%，必现 panic）
- **SC-002**: 含空段落文档成功完成索引的比例达到 100%（当前为 0%，必现 panic）
- **SC-003**: 解析失败的文档在失败后 5 秒内状态更新为 `parse_failed`，`status()` 可查询到准确状态
- **SC-004**: 格式错误的查询字符串不导致返回非零结果数（当前可能返回全库）
- **SC-005**: 连续执行 1000 次搜索后，进程文件句柄数增长不超过 10 个（相对于首次搜索后的基线）
- **SC-006**: 所有现有 Rust 单元测试和 Node 集成测试继续通过，无新增失败

---

## Assumptions

- 修复范围严格限定于上述 7 条 FR，不扩展到 LOW 级别问题（token 估算精度、`.eml` CRLF、docx 实体解码等）
- `embed_failed` 作为新的文档状态值，需同步更新 `index.d.ts` 中的类型定义，但不改变现有状态值的语义
- `IndexReader` 改为单例后，搜索期间新增的文档需通过显式 reload 或 ManualReload 策略可见，搜索实时性行为与现有一致即可
- FR-007（消除持锁 await 死锁）的具体实现方式由开发者决定（先释放锁再 await，或重构为无锁设计），不在 spec 中规定
- Windows 平台的 NULL DACL 问题（`src/tempfile.rs:69`）属于安全加固，不在本次修复范围内，记录为后续 issue
- embed 批次部分失败时（部分 chunk 成功、部分失败），已成功的 chunk 保留其 embed 结果；文档级 `embed_failed` 状态仅在**整批**失败时写入，部分失败场景不在本次修复范围，记录为后续 issue
- `IndexReader` 改为单例并使用 `ReloadPolicy::Manual` 后，`reload()` 在每次 BM25 搜索前调用，以确保新索引内容对搜索可见；此行为与现有每次新建 reader 的语义一致，性能影响在可接受范围内（验收标准：搜索延迟无可感知退步）
