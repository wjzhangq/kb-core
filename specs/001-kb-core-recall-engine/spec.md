# Feature Specification: kb-core 召回引擎

**Feature Branch**: `001-kb-core-recall-engine`

**Created**: 2026-07-18

**Status**: Draft

**Input**: User description: "按 prd-v7.md 开发项目"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 异步文档索引 (Priority: P1)

宿主进程（Electron 主进程或 Agent runtime）调用 `add(path)` 将本地文档加入知识库。调用立即返回，系统在后台异步完成解析与 embedding。纯文本类文档（.md/.txt/.eml/代码）经本地解析，图片类文档（扫描 PDF、含无文字图片的 PPTX/DOCX、纯图片）经远程解析服务转换为结构化块清单（okf）。解析完成后文档立即可被 BM25 检索；embedding 在后台异步推进，完成后向量检索也可用。

**Why this priority**: 索引是一切检索的前提，且「BM25 先行、向量最终一致」是 v7 核心架构特征，必须首先实现。

**Independent Test**: 调用 `add()` 后立即返回 docId，后台解析完成后可通过 BM25 命中该文档，`status()` 反映进度；全部 embedding 完成后向量也可命中。

**Acceptance Scenarios**:

1. **Given** 一批纯文本文档（.md/.txt/.eml），**When** 调用 `add(paths)` 后，**Then** 立即返回 `{docId, status:'pending_parse'}`，解析完成后 `status()` 显示 `parsed`，BM25 可命中，embedding 完成后 `status` 变为 `indexed`，向量可命中。
2. **Given** 一份扫描 PDF 或含无文字图片的 PPTX，**When** 调用 `add(path)` 后，**Then** 文档通过远程解析服务解析，`parsedBy='remote'`，`fromImage=true` 的 chunk 正确标记，解析完成后 BM25 可命中。
3. **Given** 单文档解析失败（本地 extractor panic 或远程服务 5xx），**When** 错误发生，**Then** 该文档 `status='parse_failed'`，`error` 字段记录原因，队列中其他文档继续处理不受影响，`status()` 健康检查报出异常。
4. **Given** `maxCpuThreads=2` + `lowThreadPriority=true`，**When** 后台索引进行中，**Then** tokio blocking pool、ONNX intra-op 线程、rayon 全局池三者均受 `maxCpuThreads` 约束，后台池线程以低优先级运行，前台 `search()` 不受干扰。

---

### User Story 2 - 混合检索（BM25 + 向量 + RRF） (Priority: P1)

宿主进程调用 `search(query, options)` 检索知识库，得到带完整来源 meta 的 chunk 列表。系统并行执行 BM25 召回和本地向量召回（multilingual-e5-small），通过 RRF 融合，返回按相关度排序的 SearchResponse。整个检索过程零出站网络请求。

**Why this priority**: 检索是 kb-core 唯一的对外价值，与索引同为核心路径。

**Independent Test**: 对已索引的中英混排文档集，用中文 query、英文术语 query、混合 query 分别检索，验证召回结果的相关性、来源 meta 完整性、`matchedBy` 标记正确性、检索延迟在目标范围内。

**Acceptance Scenarios**:

1. **Given** 已索引的中英混排文档集，**When** 使用纯英文术语 query（如 `MQTT protocol`），**Then** 相关文档被召回，`matchedBy` 正确标记命中路径（`bm25` 和/或 `vector`），`vectorCoverage` 反映当前覆盖率。
2. **Given** `bm25+vec` 模式，**When** 调用 `search()`，**Then** `SearchResponse` 包含 `results[].chunks[].pageRange`、`bbox`、`blockTypes`、`fromImage`、`charOffset`、`matchedBy`，形状恒定，不含 rerank 或 LLM 字段。
3. **Given** embedding 尚未追平（首次大批量索引进行中），**When** 调用 `search()`，**Then** `vectorCoverage<1`，`degraded` 字段说明「向量索引构建中」，BM25 正常召回，不报错。
4. **Given** `requireVector:true`，**When** 调用 `search()`，**Then** 只在 `embed_status='done'` 的 chunk 上做向量召回。
5. **Given** M2 Pro，万级 chunk，向量已追平，**When** 执行 `bm25+vec` 检索，**Then** 总延迟 ≤ 50ms。

---

### User Story 3 - 索引进度与健康检查 (Priority: P2)

宿主进程或管理界面调用 `status()` 获取当前索引进度和系统健康度，用于展示「构建中 87%」等状态，或诊断解析失败、模型未下载等问题。

**Why this priority**: 异步流水线必须有可观测性，但宿主不依赖此 API 做检索，优先级低于核心路径。

**Independent Test**: 在索引过程中多次调用 `status()`，验证进度数值单调递增，失败文档计数正确，WAL 模式状态正确，健康检查项完整。

**Acceptance Scenarios**:

1. **Given** 索引进行中，**When** 调用 `status()`，**Then** 返回 `indexed/total` 进度（BM25 已完成数量）、`vectorCoverage`（embedding 完成比例）、`parseFailedCount`、`SQLite journal_mode=WAL` 状态。
2. **Given** 存在 `parse_failed` 文档，**When** 调用 `status()`，**Then** 健康检查报出失败文档列表（docId + error 摘要）。
3. **Given** 旧库升级到 v7（chunks 缺 `char_start/char_end`，无 blocks 表），**When** 调用 `status()`，**Then** 报出「N 个文档缺来源 meta，重新索引可补全」。

---

### User Story 4 - 模型迁移与重建向量库 (Priority: P2)

使用旧默认模型（bge-small-zh / 512）建立的知识库升级到 v7 后，系统检测到 `model_tag` 不符，抛出 `KBModelMismatchError`；用户可选择调用 `reindexEmbeddings()` 用新模型重建向量库，或在配置中保留旧模型。BM25 不受影响，重建期间搜索自动降级。

**Why this priority**: 迁移影响存量用户，但不阻塞新用户，优先级低于核心检索路径。

**Independent Test**: 用 bge-small-zh 模型建库后切换到 v7 配置，验证 `KBModelMismatchError` 正确抛出；调用 `reindexEmbeddings()` 后，`model_tag` 更新，向量重建完成，`search()` 恢复正常。

**Acceptance Scenarios**:

1. **Given** 已有 bge-small-zh / 512 建立的知识库，**When** 用 v7 默认配置（multilingual-e5-small / 384）打开，**Then** 抛出 `KBModelMismatchError`，提示两条出路（reindex 或保留旧模型）。
2. **Given** `KBModelMismatchError` 后调用 `reindexEmbeddings({ name:'multilingual-e5-small', dim:384, quantization:'int8' })`，**When** 重建完成，**Then** `model_tag` 更新，`chunks_vec` 用新维度重建，`search()` 返回正常结果。
3. **Given** 重建进行中，**When** 调用 `search()`，**Then** 以 BM25 + 存量向量（覆盖率逐步提升）正常响应，不报错。

---

### Edge Cases

- 纯文本语料（无图片类文档）下，全程零出站请求：`parse.allowRemote` 配置不影响不走远程解析的文档。
- 远程解析服务熔断 open 时，图片类文档在解析队列排队等待（默认 `onRemoteParseUnavailable:'wait'`），不降级为跳过。
- `add()` 同一路径重复调用：`INSERT OR IGNORE` + 检查 `changes()`，不产生重复文档；对已存在路径返回 `{docId, status: 'already_indexed'}`，不重新触发解析。
- 邮件 mbox 文件：按 `mbox://{hash}/{message_id}` 寻址，增删邮件后序号不错位。
- NFS/SMB 上 `dataDir`：`flock` 语义不可靠，退化为 PID+starttime 探活，日志 warn。
- chunk 超过 320 token：embed 前用真实 tokenizer 校验，超长截断，`truncated=true`，计入 `truncatedChunks`。
- 附件解析：默认不索引；启用时先校验大小（`maxFileSize`）和扩展名（deny-list），递归深度上限 1。
- `add({ sync: true })`：超出本版本范围（v1 不实现），`add()` 始终异步立即返回。如未来需要同步模式，应在新 spec 中明确语义（是否等待 embedding 完成）再实现。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 系统 MUST 提供 `KnowledgeBase` 类，构造函数接收 `KBConfig` options 对象，不读配置文件和环境变量。
- **FR-002**: `add(path | path[])` MUST 立即返回 `{docId, status}` 数组，解析与 embedding 在后台异步进行。
- **FR-003**: 系统 MUST 维护每个文档的状态机：`pending_parse → parsing → parsed → indexed`（失败：`parse_failed`）。
- **FR-004**: 纯文本类文档（.md/.txt/.eml/代码等）MUST 本地解析，不出站网络请求。
- **FR-005**: 图片类文档（扫描 PDF、含无文字图片的 PPTX/DOCX、纯图片）MUST 经远程解析服务转为 okf 块清单，记录 `parsedBy='remote'`。
- **FR-006**: 解析产出 MUST 是 okf 结构化块清单（JSON 块数组，含 `blockId/type/text/page/bbox/fromImage`），而非 markdown。
- **FR-007**: BM25 索引 MUST 在文档解析完成后立即可用，不等待 embedding。
- **FR-008**: 系统 MUST 使用 `multilingual-e5-small`（384 维，int8 量化）作为默认 embedding 模型，embed 前统一注入 `query:` / `passage:` 前缀，不暴露给调用方。
- **FR-009**: `search(query, options?)` MUST 执行 BM25 + 向量并行召回，通过 RRF 融合（默认 `rrfK=60`），返回形状恒定的 `SearchResponse`，全程零出站。
- **FR-010**: `SearchResult.chunks` MUST 包含 `charOffset`、`pageRange`、`bbox`、`blockTypes`、`fromImage`、`matchedBy`、`score`，来源 meta 通过 chunk charOffset 反查 blocks 表得到。
- **FR-011**: `SearchResponse` MUST 包含 `vectorCoverage`（已 embedding chunk / 总 chunk）和 `timing`（各阶段耗时细分）；向量覆盖不足时通过 `degraded` 字段说明。
- **FR-012**: 系统 MUST 同时约束三个线程池（tokio blocking pool、ONNX intra-op、rayon global pool）不超过 `maxCpuThreads`（默认 2），后台池线程以低优先级运行，且低优先级不可逆（前台池与后台池物理隔离）。
- **FR-013**: `status()` MUST 返回索引进度、向量覆盖率、解析失败文档列表、SQLite WAL 模式状态。
- **FR-014**: 系统 MUST 使用 `flock`（`fs4` crate）实现 writer 互斥，进程退出（含 kill -9）后锁自动释放，不做 PID 探活；`.writer.lock` 文件仅存人类可读诊断信息。
- **FR-015**: 单文档解析或 embedding 失败 MUST 只影响该文档，不阻塞队列，不影响宿主进程；所有 extractor 强制 `catch_unwind`。
- **FR-016**: `reindexEmbeddings(model)` MUST 检测 `model_tag` 不符时抛出 `KBModelMismatchError`，重建向量库期间 `search()` 以 BM25 + 存量向量降级正常响应。
- **FR-017**: 系统 MUST 在进程启动时清空 `{dataDir}/tmp/` 残留，附件解出前校验大小和扩展名，临时文件使用 `tempfile` crate（Unix 权限 0600，Windows `FILE_ATTRIBUTE_TEMPORARY`）。
- **FR-018**: SQLite 连接 MUST 强制 `journal_mode=WAL`；tantivy reader 按 `readerReloadInterval`（默认 5s）刷新，支持多进程只读并发。
- **FR-019**: 远程解析服务 MUST 走三态熔断保护；熔断 open 期间按 `onRemoteParseUnavailable`（默认 `wait`）处理，不静默丢内容。
- **FR-020**: 系统 MUST 对外唯一形态为 Node API（宿主内嵌），无 CLI，无独立 serve 进程，不含 rerank，不含 `answer()` / LLM 接口。

### Key Entities

- **KnowledgeBase**: 知识库实例，持有 SQLite 数据库、tantivy 索引、embedding 模型、解析队列和 embedding 队列。
- **OkfBlock**: 结构化块，含 blockId、type、text、page?、bbox?、fromImage?；blocks 数组构成一份文档的 okf。
- **Chunk**: 文档切分单元，含 charOffset（派生线性文本坐标）、embed_status、truncated；与 blocks 表通过区间相交关联。
- **SearchResult**: 文档级检索结果，含 docId、path、score、chunks 数组（每个 chunk 带完整来源 meta）。
- **KBConfig**: 构造函数配置，含 InferenceConfig（`bm25-only` / `local-first` / `remote`）、SystemConfig（线程约束）、ProcessingConfig（分块参数）。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 万级 chunk（约 1500 篇中等文档，纯文本类），在主流办公本（i5-1235U）避让模式下，完整索引（含 embedding）在 90 分钟内完成；首批文档在 `add()` 后数分钟内可被 BM25 检索，无需等待全部索引完成。
- **SC-002**: 在 M2 Pro，万级 chunk，向量已追平时，`bm25+vec` 模式检索延迟 ≤ 50ms（端到端，含 query embedding）。
- **SC-003**: 中英混排文档集上，纯英文术语 query（如 `BLE`、`MQTT`、`OIDC`）可被语义路正确召回，召回率不低于中文 query 同类测试。
- **SC-004**: 纯文本语料（无图片类文档）全程零出站网络请求（可通过网络监控验证）。
- **SC-005**: 单文档解析失败时，其余文档继续正常索引，宿主进程不崩溃，故障文档通过 `status()` 可见。
- **SC-006**: 后台索引进行中，前台 `search()` 响应时间不因后台 CPU 占用而超过 SC-002 目标的 2 倍（即 ≤ 100ms）。
- **SC-007**: 所有 `SearchResult.chunks` 均含有效的 `matchedBy`、`charOffset`；来源 meta（`pageRange`/`blockTypes`/`fromImage`）对解析完整的文档 100% 可回填。

## Assumptions

- 宿主进程负责将 `search()` 结果包装为 MCP tool、HTTP 端点或其他对外形态；kb-core 不关心上层协议。
- 远程解析服务由部署方自建或选用第三方，兼容 `POST {endpoint}/v1/parse` 契约（返回 okf 块清单）；kb-core 不捆绑具体解析服务实现。
- `multilingual-e5-small` int8 ONNX 模型文件（~55MB）在 `npm install` 时由 `postinstall` 脚本自动预下载到包内 `models/` 目录；宿主也可通过 `modelsDir` 选项指定预置路径。模型不随 npm 包源码分发，下载失败时仅 warn，构造 `KnowledgeBase` 时若模型缺失则抛 `ModelNotFoundError`。
- 目标语料以中英混排技术文档、邮件为主；纯中文语料用户可通过配置切换到 `bge-small-zh-v1.5`。
- 性能数据基于 PRD §15 给出的基准机器，实际性能因语料大小、文件类型（含图片类文档比例）和硬件差异而异。
- 上层（宿主 / Agent）若需 rerank 或 RAG 问答，自行在 `search()` 结果上对接；kb-core 不提供此能力。
- 开发语言为 Rust（napi-rs），对外暴露 Node.js API；已有骨架代码（tantivy chunk 层，13/13 测试通过）。
