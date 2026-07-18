# Tasks: kb-core 召回引擎

**Input**: Design documents from `specs/001-kb-core-recall-engine/`

**Prerequisites**: plan.md ✅ spec.md ✅ research.md ✅ data-model.md ✅ contracts/node-api.md ✅ quickstart.md ✅

**Tests**: 本 spec 未明确要求 TDD，测试任务仅包含 PRD §14 明确指定的「新需要钉住的」集成测试用例。

**Organization**: 按用户故事分组，支持独立实现与验证。

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: 可并行（不同文件，无未完成依赖）
- **[Story]**: 所属用户故事（US1–US4）
- 包含精确文件路径

---

## Phase 1: Setup（项目初始化）

**Purpose**: 初始化 napi-rs 项目骨架，配置构建工具链和 CI 框架。

- [ ] T001 初始化 napi-rs 项目：生成 `Cargo.toml`（workspace + napi crate）、`package.json`（入口、scripts: build/test/postinstall）、`build.rs`、`.npmignore`
- [ ] T002 [P] 创建 `index.js`（平台 .node 加载器）和 `index.d.ts`（公开 TypeScript 类型声明，含所有 contracts/node-api.md 中的接口）
- [ ] T003 [P] 创建 `models/.gitkeep` 并在 `.gitignore` 中排除 `models/` 目录下的 ONNX 文件
- [ ] T004 创建 `.github/workflows/ci.yml`：PR/push 触发，ubuntu-latest，`cargo test` + `cargo clippy --deny warnings` + `npm run build:debug` + `npx vitest run`
- [ ] T005 [P] 创建 `.github/workflows/publish.yml`：`push: tags: ['v*']` 触发，5 平台 matrix 构建预编译 `.node`，收集后 `npm publish --access public`（需 secret `NPM_TOKEN`）
- [ ] T006 [P] 创建 `src/lib.rs`（napi-rs 导出入口，初始为空导出）和目录骨架：`src/db/`、`src/tantivy_idx/`、`src/pipeline/`、`src/parse/`、`src/embed/`、`src/search/`
- [ ] T007 [P] 创建 `tests/rust/`（空 mod）和 `tests/node/`（vitest 配置 `vitest.config.ts`）

---

## Phase 2: Foundational（所有用户故事的阻塞性前置）

**Purpose**: 核心基础设施——配置类型、SQLite 迁移、线程池约束、flock 锁、临时文件安全。全部完成前任何 User Story 均不可开始。

**⚠️ CRITICAL**: 以下任务必须在 Phase 3+ 开始前全部完成。

- [ ] T008 实现 `src/config.rs`：`KBConfig`、`InferenceConfig`（3 个 variant）、`EmbeddingModelSpec`、`SystemConfig`、`ProcessingConfig`、`RemoteParseConfig`、`BreakerConfig` 完整类型定义（对应 data-model.md §Rust 类型定义）；napi-rs `#[napi(object)]` 导出到 JS
- [ ] T009 实现 `src/db/migrations.rs`：嵌入式 5 版本迁移（`include_str!` 内嵌 SQL），启动时按序执行缺失迁移；`src/db/schema.rs` 定义表名/列名常量
- [ ] T010 [P] 创建迁移 SQL 常量（内嵌于 migrations.rs）：`001_initial`（documents/chunks/kb_meta）、`002_tantivy_meta`、`003_vec`（chunks_vec float[384]）、`004_wal`（WAL pragma）、`005_v7_async_and_okf`（blocks 表、char_start/char_end、embed_status、parsed_by、status 字段）
- [ ] T011 实现 `src/thread_pool.rs`：三线程池约束——`build_tokio_runtime(max_threads)`、`configure_rayon(max_threads)`（`ThreadPoolBuilder::build_global`）、`demote_current_thread()`（Linux: `setpriority(gettid, 10)`；macOS: `pthread_set_qos_class_self_np(UTILITY)`；Windows: `SetThreadPriority(BELOW_NORMAL)`）；前台池与后台池物理隔离
- [ ] T012 实现 `src/lock.rs`：`flock` advisory lock（`fs4::FileExt::try_lock_exclusive`），`.writer.lock` 写入 PID+starttime+hostname 仅供诊断；`statfs` 探测 NFS/SMB 时退化为 PID+starttime 探活 + warn；返回 `KBLockError`（含 `held_by`）
- [ ] T013 实现 `src/tempfile.rs`：`tempfile::Builder` 创建 0600（Unix）/ `FILE_ATTRIBUTE_TEMPORARY`（Windows）临时文件；`acl-restricted` 模式下 Windows `SetNamedSecurityInfoW` 剥离 Everyone/Guests ACE（`windows-sys` feature gate）；进程启动时清空 `{dataDir}/tmp/` 残留
- [ ] T014 实现 `src/db/mod.rs`：`DbConn` 封装——强制 `PRAGMA journal_mode=WAL`、`PRAGMA foreign_keys=ON`；`open_writer(data_dir)` 获取 flock 后打开并迁移；`open_readonly(data_dir)` 不取锁只读

**Checkpoint**: Foundation 完成——所有 User Story 实现可以并行启动。

---

## Phase 3: User Story 1 — 异步文档索引（Priority: P1）🎯 MVP

**Goal**: `add(path[])` 立即返回，后台两段异步队列完成解析与 embedding，BM25 先行、向量最终一致。

**Independent Test**: 调用 `add()` 立即返回 docId；轮询 `status()` 至 `parsed`，用 BM25 命中文档；再等 `indexed`，向量也命中。见 quickstart.md §3。

### 集成测试（US1）

- [ ] T015 [P] [US1] 实现 `tests/rust/test_pipeline.rs`：钉住 `async_bm25_before_vector`——文档解析后 BM25 立即可命中，`embed_status=0` 时向量路返回空，`vectorCoverage<1`
- [ ] T016 [P] [US1] 实现 `tests/rust/test_pipeline.rs`：钉住 `remote_parse_only_for_image_docs`——纯文本文档 `parsed_by='local'`、无出站；图片类文档 `parsed_by='remote'`、`fromImage` 正确标记

### 实现（US1）

- [ ] T017 [US1] 实现 `src/parse/local.rs`：本地 extractor 分流——`.md/.txt/.eml/code` 解析为 okf 块清单；`.pdf` 文本层提取（pdfium）并按 `textLayerThreshold` 判断是否送远程；`.docx/.pptx` 无图片时本地解析；所有 extractor 包 `catch_unwind`
- [ ] T018 [US1] 实现 `src/parse/remote.rs`：HTTP 客户端（reqwest）向 `POST {endpoint}/v1/parse` 发送 multipart；解析返回 okf；三态熔断（CLOSED/OPEN/HALF-OPEN，failure_threshold=5，reset_timeout_ms=30000）；`onRemoteParseUnavailable: wait|text-only|skip`；上传前校验 maxFileSize + deny-list
- [ ] T019 [US1] 实现 `src/pipeline/parse.rs`：解析队列（tokio channel + 后台池）——`drain pending_parse`、分流调用 local/remote extractor、产出 okf、派生线性文本（`\n\n` 分隔块 text）落库（blocks 表 lin_start/lin_end）、chunk 切分（尊重块边界，overlap_sentences 块内生效）、BM25 写入（tantivy writer）、`embed_status=0`；`status='parsed'`；`parseConcurrency` 控制并发上限；单文档失败 → `status='parse_failed'`，不阻塞队列
- [ ] T020 [US1] 实现 `src/embed/e5.rs`：fastembed `TextEmbedding` 封装，`SessionBuilder::with_intra_threads(max_cpu_threads).with_inter_threads(1)`；`query:` / `passage:` 前缀注入（不暴露给调用方）；batch embed；`tokenizers` 校验 chunk 长度（超 320 token 截断 + `truncated=true`）；模型路径从 `modelsDir`（默认 `<package>/models/multilingual-e5-small/`）读取；模型缺失时返回 `ModelNotFoundError`
- [ ] T021 [US1] 实现 `src/pipeline/embed.rs`：embedding 队列（tokio channel + 后台池）——`drain embed_status=0`、batch 16 推理、写 `chunks_vec`、`embed_status=1`（失败=2）；所有 chunk done 后 `documents.status='indexed'`；`embed_status=3` for bm25-only 模式
- [ ] T022 [US1] 实现 `src/lib.rs` 中的 `KnowledgeBase::add()` napi 方法：登记 documents 行（`INSERT OR IGNORE`）、放入解析队列、立即返回 `AddResult[]`；启动时初始化两段后台队列（各自独立 tokio 任务 + 低优先级线程池）
- [ ] T023 [US1] 创建 `scripts/download-model.js`：下载 multilingual-e5-small int8 ONNX 三文件（`model_quantized.onnx`、`tokenizer.json`、`tokenizer_config.json`）到 `models/multilingual-e5-small/`；SHA256 校验；文件已存在跳过；失败只 warn 不 throw；`package.json` scripts.postinstall 指向此脚本

**Checkpoint**: User Story 1 完成——`add()` 异步索引端到端可验证，BM25+向量均可命中。

---

## Phase 4: User Story 2 — 混合检索 BM25+向量+RRF（Priority: P1）

**Goal**: `search()` 并行 BM25 + 向量召回，RRF 融合，返回带完整来源 meta 的 `SearchResponse`，全程零出站。

**Independent Test**: 中英混排语料，用英文术语 query，验证召回、`matchedBy`、`charOffset`、`blockTypes` 均正确，延迟 ≤ 50ms（M2 Pro）。见 quickstart.md §4–§5。

### 集成测试（US2）

- [ ] T024 [P] [US2] 实现 `tests/rust/test_search.rs`：钉住 `cjk_english_mixed_recall`——中英混排文档 + 纯英文术语 query（`BLE`、`MQTT`），向量路命中，`matchedBy` 包含 `vector`
- [ ] T025 [P] [US2] 实现 `tests/rust/test_meta.rs`：钉住 `chunk_to_block_offset_lookup`——chunk `[char_start, char_end)` 与 blocks `idx_blocks_span` 区间相交，正确聚合 `pageRange`/`blockTypes`/`fromImage`

### 实现（US2）

- [ ] T026 [US2] 更新 `src/tantivy_idx/schema.rs`（已有骨架，13/13 测试通过，在原有基础上修改）：确认 chunk 级 schema 字段完整（chunk_id/doc_id/doc_type/text/title/path）；补注册自定义 tokenizer（`tantivy-jieba` 中文分词 + `SimpleTokenizer` 小写化英文），确保原有测试仍通过
- [ ] T027 [US2] 更新 `src/tantivy_idx/writer.rs`（已有骨架，在原有基础上修改）：将 `index.writer(budget)` 改为显式 `writer_with_num_threads(max_cpu_threads, budget)`；确认批量 commit（200 doc / 1000ms）和 delete-then-add opstamp 语义已正确实现；`close()` 幂等；原有钉住测试（`reindex_within_single_commit`、`delete_document_removes_all_its_chunks`）必须继续通过
- [ ] T028 [US2] 实现 `src/search/bm25.rs`：tantivy query 解析（syntax: text/fielded/raw，text 模式全转义防冒号等特殊字符）；topK chunk 级召回；`reader.reload()` 按 `readerReloadInterval`（默认 5s）刷新
- [ ] T029 [US2] 实现 `src/search/vector.rs`：sqlite-vec `vec_distance_cosine` + `ORDER BY LIMIT topK` 全量扫描；仅在 `embed_status=1` 的 chunk 上召回；`requireVector=true` 时过滤；`vectorCoverage` 计算
- [ ] T030 [US2] 实现 `src/search/rrf.rs`：RRF 融合（`score = Σ 1/(rrfK + rank_i)`，默认 `rrfK=60`）；**保留每个 chunk 的 `matchedBy`**（命中了哪条路，可同时含 bm25+vector）；去重（同 chunk_id 合并）
- [ ] T031 [US2] 实现 `src/search/meta.rs`：chunk `[char_start, char_end)` 与 `blocks(doc_id, lin_start, lin_end)` 区间相交（`idx_blocks_span`）；聚合 `pageRange`（min/max page）、`bbox`（array of {page, rect}）、`blockTypes`（去重排序）、`fromImage`（任意块为 true 则 true）
- [ ] T032 [US2] 实现 `src/search/mod.rs`：完整检索管线（§10.1 6 步）——query 解析 + e5 前缀注入 → BM25/向量并行召回 → RRF 融合 → meta 反查 → chunk→doc 聚合（max/sum/top2sum）→ topN；`SearchOptions` 全字段支持（filter/syntax/includeText/maxCharsPerChunk/requireVector/signal）；`timing` 各阶段计时；降级矩阵（bm25-only/首次索引中/模型未下载）
- [ ] T033 [US2] 在 `src/lib.rs` 实现 `KnowledgeBase::search()` napi 方法：调用 `src/search/mod.rs`，序列化 `SearchResponse` 为 napi 对象
- [ ] T034 [P] [US2] 实现 `tests/node/add-and-search.test.ts`（Vitest）：`add()` → 等 parsed → BM25 搜 → 等 indexed → 向量搜；验证 `SearchResponse` 形状完整，无 rerank/LLM 字段

**Checkpoint**: User Story 2 完成——P1 核心路径全部可验证，可作为 MVP 发布。

---

## Phase 5: User Story 3 — 索引进度与健康检查（Priority: P2）

**Goal**: `status()` 返回完整进度快照和健康告警，宿主可展示进度条并诊断问题。

**Independent Test**: 索引进行中多次调用 `status()`，验证数值单调递增、失败文档可见、WAL 状态正确。见 quickstart.md §6。

### 实现（US3）

- [ ] T035 [US3] 实现 `src/lib.rs` 中的 `KnowledgeBase::status()` napi 方法：查询 documents 各 status 计数、chunks embed_status 计数、`vectorCoverage`、`PRAGMA journal_mode` 验证、writer lock 状态、模型文件可达性；组装 `KBStatus` 含 `warnings[]`（parse_failed / missing_meta / model_not_found / wal_disabled）
- [ ] T036 [P] [US3] 实现 `tests/node/status.test.ts`（Vitest）：parse_failed 文档出现在 warnings；旧库升级（缺 blocks/charOffset）时 `missing_meta` 告警；`walEnabled=true`

**Checkpoint**: User Story 3 完成——健康检查独立可用。

---

## Phase 6: User Story 4 — 模型迁移与重建向量库（Priority: P2）

**Goal**: 旧库升级触发 `KBModelMismatchError`；`reindexEmbeddings()` 重建向量库，降级期间 `search()` 正常响应。

**Independent Test**: bge-small-zh 建库后换 multilingual-e5-small 配置，验证错误抛出 → 重建 → 搜索恢复。见 quickstart.md §7。

### 实现（US4）

- [ ] T037 [US4] 在 `src/db/migrations.rs` 中实现 `model_tag` 校验逻辑：启动时对比 `kb_meta.model_tag` 与当前配置计算的 `sha256("{name}|{dim}|{quantization}")[..16]`；不符则抛 `KBModelMismatchError`（含 expected/found 字段），不自动重建
- [ ] T038 [US4] 在 `src/lib.rs` 实现 `KnowledgeBase::reindex_embeddings()` napi 方法：清空 `chunks_vec`、重置全部 `embed_status=0`、更新 `kb_meta.model_tag`、触发 embedding 队列重处理；重建期间 `search()` 以 BM25+存量向量正常响应，`degraded` 字段说明「向量库重建中」
- [ ] T039 [P] [US4] 实现 `tests/node/model-migration.test.ts`（Vitest）：`KBModelMismatchError` 类型与字段；`reindexEmbeddings()` 后 `model_tag` 更新，`search()` 恢复正常；重建中 `search()` 不报错

**Checkpoint**: User Story 4 完成——迁移路径端到端可验证。

---

## Phase N: Polish & Cross-Cutting Concerns

**Purpose**: 跨故事优化、文档完善、发布准备。

- [ ] T040 [P] 实现 `src/lib.rs` 中的 `KnowledgeBase::close()` napi 方法：等待当前 batch 完成（不等全队列）、tantivy commit、SQLite close、flock 释放；多次调用幂等
- [ ] T041 [P] 补充 `index.d.ts` 中的 `KBLockError` 和 `KBModelMismatchError` 类型导出（含 `heldBy`、`expected`、`found` 字段）
- [ ] T042 [P] 编写 `README.md`：安装（含 postinstall 模型下载说明）、快速上手代码示例、选型矩阵（§2.2）、`KBConfig` 配置参考、`onModelDownloadRequired` 处理指引
- [ ] T043 运行完整 quickstart.md 验证场景（§2–§9），确认全部通过；跑 `cargo test` + `vitest run` 确认 0 失败
- [ ] T044 [P] 配置 `package.json` 的 `files` 字段（只包含 `index.js`、`index.d.ts`、`models/`、`*.node`、`scripts/`），确认 `.npmignore` 排除 `src/`、`tests/`、`specs/`
- [ ] T045 [P] 在 `publish.yml` 中验证 matrix 产物命名规范（`kb-core.{platform}-{arch}.node`）并与 `index.js` 加载逻辑一致；设置 `package.json` 的 `optionalDependencies` 平台子包字段

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: 无依赖，立即开始
- **Foundational (Phase 2)**: 依赖 Phase 1 完成，**阻塞所有 User Story**
- **US1 (Phase 3)**: 依赖 Phase 2 完成；无其他 Story 依赖
- **US2 (Phase 4)**: 依赖 Phase 2 完成；部分依赖 US1（tantivy writer 和 pipeline 已在 US1 实现）
- **US3 (Phase 5)**: 依赖 Phase 2 完成；独立于 US1/US2（status 查询 DB，不依赖检索路径）
- **US4 (Phase 6)**: 依赖 Phase 2 + US1（embed pipeline）完成
- **Polish (Phase N)**: 依赖所有需要的 User Story 完成

### User Story Dependencies

- **US1 (P1)**: Phase 2 完成后可立即开始，无 Story 间依赖
- **US2 (P1)**: Phase 2 完成后可立即开始；T026/T027（tantivy schema/writer）与 US1 无冲突，可并行；T028–T033 依赖 US1 的 embed 路径已就绪
- **US3 (P2)**: Phase 2 完成后可立即开始，独立于 US1/US2
- **US4 (P2)**: 依赖 US1 的 embed pipeline（T020/T021）已完成

### Within Each User Story

- 集成测试任务（标 [P]）可与实现任务并行开始（先写测试使其 fail，再实现）
- pipeline（T019）依赖 local.rs（T017）和 remote.rs（T018）
- search/mod.rs（T032）依赖 bm25/vector/rrf/meta（T028–T031）

### Parallel Opportunities

| 阶段 | 可并行任务 |
|------|-----------|
| Phase 1 | T002/T003/T005/T006/T007（Setup 中标 [P] 的任务同时进行） |
| Phase 2 | T009+T010 同时进行；T011/T012/T013 三个任务并行；T014 依赖 T009 |
| Phase 3 (US1) | T015/T016 测试与 T017/T018 实现并行开始 |
| Phase 4 (US2) | T024/T025 测试与 T026/T027 并行；T028/T029 与 T030/T031 并行 |
| Phase 5 (US3) | T035/T036 可同时进行 |
| Phase 6 (US4) | T037/T038 顺序；T039 可与 T038 并行 |

---

## Parallel Example: User Story 1

```bash
# 并行启动（不同文件，无依赖冲突）
Task: "T015 test_pipeline.rs: async_bm25_before_vector"
Task: "T016 test_pipeline.rs: remote_parse_only_for_image_docs"
Task: "T017 src/parse/local.rs"
Task: "T018 src/parse/remote.rs"

# 顺序（T019 依赖 T017/T018）
Task: "T019 src/pipeline/parse.rs"

# 并行（T020/T021 互不依赖）
Task: "T020 src/embed/e5.rs"
Task: "T021 src/pipeline/embed.rs"  # 可与 T020 并行，完成后对接

# 最后
Task: "T022 src/lib.rs: add()"
Task: "T023 scripts/download-model.js"
```

---

## Implementation Strategy

### MVP First（US1 + US2，P1 核心路径）

1. 完成 Phase 1: Setup
2. 完成 Phase 2: Foundational（**关键**，阻塞所有 Story）
3. 完成 Phase 3: US1（异步索引）
4. **STOP & VALIDATE**: `add()` 端到端验证（quickstart.md §3）
5. 完成 Phase 4: US2（混合检索）
6. **STOP & VALIDATE**: 中英混排检索验证（quickstart.md §4–§5）
7. → **MVP 可发布**（`npm publish`）

### Incremental Delivery

1. Setup + Foundational → 基础设施就绪
2. US1 → BM25+向量 索引可用 → Demo: `add()` + 简单搜索
3. US2 → 完整混合检索 + 来源 meta → **MVP**
4. US3 → 进度监控可用 → 宿主可展示进度条
5. US4 → 迁移路径可用 → 存量用户升级

### Team Parallel Strategy

有多人开发时：
1. 全员完成 Phase 1+2
2. Phase 2 完成后：
   - 开发者 A：US1（T015–T023）
   - 开发者 B：US3（T035–T036，独立）
   - Phase 4 US2 等 US1 的 embed pipeline 完成后启动

---

## Notes

- `[P]` = 不同文件，当前阶段内无未完成依赖，可并行
- `[USn]` 标签将任务追溯到 spec.md 中对应的用户故事
- 每个 User Story 阶段结束时均有 Checkpoint，可独立验证再继续
- 模型下载（T023）是 postinstall 脚本，开发期间需手动执行一次或设置 `KB_MODELS_DIR`
- napi-rs `.node` 产物需先 `npm run build:debug` 才能跑 Vitest Node 集成测试
