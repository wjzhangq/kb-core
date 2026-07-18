# Research: kb-core 召回引擎

## 1. napi-rs 跨平台预编译二进制发布

**Decision**: 使用 napi-rs `@napi-rs/cli` 的标准跨平台构建方案，通过 GitHub Actions matrix 在多平台编译，产物作为 `optionalDependencies` 发布到 npm（`kb-core-darwin-arm64`、`kb-core-darwin-x64`、`kb-core-linux-x64-gnu` 等子包）。

**Rationale**: napi-rs 官方 scaffold 已内建此模式，`@napi-rs/cli build` 输出平台特定的 `.node` 文件；`index.js` 在运行时按 `process.platform + process.arch` 选择加载。相比手动管理 node-pre-gyp，维护成本极低，且 GitHub Actions matrix + `napi-rs/publish` action 是生态标准。

**Alternatives considered**:
- node-gyp + node-pre-gyp：构建脚本复杂，跨平台矩阵维护负担重，社区已逐渐迁移到 napi-rs。
- WASM：无法直接访问本地文件系统，排除。

**Key facts**:
- napi-rs 生成的 `package.json` 的 `optionalDependencies` 字段列出所有平台子包，npm install 时只拉取当前平台的二进制。
- GitHub Actions `napi-rs/napi-rs/.github/workflows/` 提供可复用 matrix 模板：`ubuntu-latest`（gnu/musl）、`macos-latest`（arm64）、`macos-13`（x64）、`windows-latest`（x64）。
- 发布前需先 `napi prepublish` 更新子包版本号，再逐平台上传，最后 `napi-rs publish` 发布主包。

---

## 2. multilingual-e5-small postinstall 模型下载

**Decision**: `package.json` 的 `scripts.postinstall` 执行 `node scripts/download-model.js`，从 HuggingFace（或镜像）下载 `multilingual-e5-small` int8 ONNX 模型（`model.onnx` + `tokenizer.json`，共约 55-60MB）到包内 `models/` 目录。

**Rationale**:
- 模型不随 npm 包分发（GitHub npm registry 有 100MB 包大小限制，npm 主站限制 256MB，55MB 勉强但加上二进制 + tokenizer 风险较高）。
- postinstall 是 npm 生态的标准模型分发位置（同类方案：`@tensorflow/tfjs-node`、`onnxruntime-node` 的模型下载脚本）。
- `CI=true` 环境下 postinstall 失败不应阻断安装（脚本退出码 0），模型缺失时 `KnowledgeBase` 构造时给出明确错误信息。

**Script behavior**:
1. 检查 `models/multilingual-e5-small/model_quantized.onnx` 是否已存在（跳过重复下载）。
2. 从 `https://huggingface.co/intfloat/multilingual-e5-small/resolve/main/onnx/model_quantized.onnx` 下载。
3. 同时下载 `tokenizer.json`、`tokenizer_config.json`、`special_tokens_map.json`。
4. 校验文件 SHA256（hardcode 在脚本里），不匹配则删除并重试一次。
5. 下载失败时打印警告（不 throw），提示用户手动放置或设置 `KB_MODELS_DIR` 环境变量。

**Alternatives considered**:
- 将模型放入 `optionalDependencies` 的专属包：模型会随每个 npm install 下载，无法跳过；且模型更新与包版本解耦更灵活。
- 运行时首次下载（懒加载）：体验差，用户第一次 `add()` 时卡住数十秒无反馈；排除。
- 允许宿主预置 `modelsDir`：保留此选项作 escape hatch（spec Assumptions），但默认仍走 postinstall。

---

## 3. fastembed + ONNX Runtime：intra_threads 约束

**Decision**: 使用 `fastembed` 5.x（Rust crate），在 `SessionBuilder` 上显式设置 `with_intra_threads(maxCpuThreads)` 和 `with_inter_threads(1)`，防止 ONNX Runtime 默认吃满物理核。

**Rationale**: PRD §7.3 明确指出 ONNX Runtime intra-op 是「最容易漏的、最吃 CPU 的」线程池。fastembed 5.x 暴露了 `ort::SessionBuilder` 的配置接口；`inter_threads(1)` 防止多个算子组并行（对批量 embed 无明显收益但额外占核）。

**Key implementation detail**: `fastembed::TextEmbedding::try_new_with_model` 接受自定义 `InitOptions`，其中 `ExecutionProviders` 和 `MaxLength` 可配；需要直接构建 `ort::Session` 才能控制线程数，fastembed 5.x 支持此路径。

---

## 4. sqlite-vec 全量暴力扫描（万级 chunk）

**Decision**: 对 `chunks_vec` 虚表使用 `vec_distance_cosine` + `ORDER BY ... LIMIT topK` 全量扫描，不使用 HNSW 近似索引。

**Rationale**: PRD §10.1 明确「float32 全量暴力扫描，万级约 3ms」。sqlite-vec 的 `vec0` 虚表在万级规模下全量扫描耗时可忽略（3ms vs. query embed 35ms），引入 HNSW 增加实现复杂度但无显著收益。规模扩展到百万级时可切换，但 PRD 明确「不为将来预付成本」。

---

## 5. GitHub Actions：CI + 发布 Workflow

**Decision**:
- `ci.yml`：PR/push 触发，单平台（ubuntu-latest）`cargo test` + `cargo clippy`，构建 `.node` 产物后运行 Vitest 集成测试。
- `publish.yml`：`push: tags: ['v*']` 触发，matrix 构建全平台预编译二进制，上传到 GitHub Release，最后 `npm publish`。

**Key steps in publish.yml**:
1. matrix: `[ubuntu-latest, ubuntu-22.04-arm64, macos-latest, macos-13, windows-latest]`
2. 每个 job：`napi build --platform --release` → 上传 `*.node` artifact
3. 汇总 job：收集所有 `.node` → `napi prepublish -t npm` → `npm publish --access public`
4. Secrets 需要：`NPM_TOKEN`（npm publish）

**napi-rs version pin**: 使用 `@napi-rs/cli@3.x` 与 `napi@3.x` 保持一致。

---

## 6. tantivy tokenizer：中英混排

**Decision**: tantivy 的 `TokenizerManager` 注册自定义 tokenizer：`jieba`（中文分词）+ `SimpleTokenizer` 小写化（英文 token），通过 `tantivy-jieba` crate 组合。

**Rationale**: PRD §2.4 要求「BM25 路的中英混合分词是硬要求：中文分词 + 英文 token 小写化」。`tantivy-jieba` 已有成熟实现，直接集成。

---

## 7. 三态熔断器

**Decision**: 自实现轻量三态熔断（CLOSED/OPEN/HALF-OPEN），不引入额外 crate（`failsafe-rs` 等过重）。

**State transitions**:
- CLOSED → OPEN：连续 N 次失败（`failureThreshold`，默认 5）
- OPEN → HALF-OPEN：超过 `resetTimeout`（默认 30s）后第一个请求
- HALF-OPEN → CLOSED：成功；HALF-OPEN → OPEN：失败

计入失败：网络错误、超时、5xx、embed 返回维度不符。4xx 不计入。

---

## 8. SQLite 迁移策略

**Decision**: 嵌入式迁移，Rust 启动时按版本号顺序执行缺失迁移。迁移文件内嵌为常量字符串（`include_str!`），无需外部 SQL 文件。

迁移版本：
- `001_initial.sql`：基础表（documents, chunks, kb_meta）
- `002_tantivy_meta.sql`：tantivy model_tag
- `003_vec.sql`：chunks_vec 虚表（dim 384）
- `004_v6.sql`：多进程读并发相关（WAL pragma 确认）
- `005_v7_async_and_okf.sql`：blocks 表、char_start/char_end、embed_status、parsed_by、status 字段
