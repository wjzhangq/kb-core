# Implementation Plan: kb-core 召回引擎

**Branch**: `001-kb-core-recall-engine` | **Date**: 2026-07-18 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/001-kb-core-recall-engine/spec.md`

## Summary

构建 `kb-core` npm 包：基于 Rust（napi-rs）的本地优先召回引擎，支持 BM25 全文检索 + multilingual-e5-small 本地向量检索 + RRF 融合。两段异步流水线（解析队列 + embedding 队列），纯文本类文档本地解析，图片类文档经远程解析服务转为结构化块清单（okf）。唯一对外形态为 Node API，宿主进程内嵌。通过 GitHub Actions 发布到 npm，预下载 multilingual-e5-small int8 ONNX 模型（~55MB）到包内。

## Technical Context

**Language/Version**: Rust 1.78+（napi-rs 2.x）+ Node.js 18+ LTS

**Primary Dependencies**:
- `napi` / `napi-derive` 2.x — Rust→Node FFI
- `tantivy` 0.22 — BM25 全文索引
- `rusqlite` 0.31 + `sqlite-vec` — 向量存储虚表
- `fastembed` 5.x（+ `ort` ORT backend）— multilingual-e5-small 本地推理
- `tokenizers` 0.19（HuggingFace）— 真实 tokenizer 校验 chunk 长度
- `fs4` 0.12 — 跨平台 flock advisory lock
- `reqwest` 0.12 — 远程解析 HTTP 客户端
- `tokio` 1.x — 异步运行时
- `rayon` 1.x — tantivy 并行（受 maxCpuThreads 约束）
- `tempfile` 3.x — 安全临时文件
- `libc` 0.2 / `windows-sys` 0.59 — 线程优先级 API

**Storage**: SQLite（单文件 `{dataDir}/kb.db`，WAL 模式）+ tantivy 索引目录（`{dataDir}/tantivy/`）

**Testing**: `cargo test`（Rust 单元 + 集成）+ Vitest（Node.js 集成测试，跑 `.node` 构建产物）

**Target Platform**: macOS（x64/arm64）、Linux（x64/arm64）、Windows（x64）

**Project Type**: npm 库（native addon，napi-rs 预编译二进制）

**Performance Goals**: bm25+vec 检索 ≤ 50ms（M2 Pro，万级 chunk）；后台避让模式 embedding ≤ 140ms/chunk（主流办公本）

**Constraints**: `maxCpuThreads` 默认 2；三线程池（tokio blocking / ONNX intra-op / rayon）均不超此限；后台池低优先级不可逆

**Scale/Scope**: 目标万级 chunk（约 1500 篇中等文档）；首个可用版本

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### 原则 I · 范围纪律 Gate

逐条核对新增项是否对应 spec/PRD 的明确要求：

| 新增项 | 对应 spec/PRD 条目 | 状态 |
|--------|------------------|------|
| GitHub Actions 发布 npm | 用户输入（plan args）+ 标准库发布需求 | ✅ 在范围内 |
| 模型默认下载（postinstall） | 用户输入（plan args）；spec Assumptions 提及「首次使用时按需下载，或由宿主预置」→ 用户明确选择预下载 | ✅ 在范围内 |
| napi-rs 跨平台预编译二进制 | FR-020（Node API 宿主内嵌）；npm 库的标准分发方式 | ✅ |
| tantivy / rusqlite / sqlite-vec | FR-007/FR-009/FR-018；PRD §9 数据模型明确指定 | ✅ |
| fastembed + ONNX | FR-008（multilingual-e5-small 本地推理） | ✅ |
| 熔断器（三态） | FR-019 + PRD §10.3 明确要求 | ✅ |
| WAL + flock | FR-014/FR-018 | ✅ |
| Vitest Node 集成测试 | 测试是 spec 验收场景的执行方式，在范围内 | ✅ |

**无未追溯到 spec 的新增项。Gate 通过。**

### 原则 II · 主动澄清 Gate

PRD v7 对所有方向性决策已有明确拍板（模型、架构、接口形态、出网策略）。用户在 plan 参数中补充了两个决策点（GitHub Actions 发布 + 模型预下载），无需进一步澄清。

**无未决方向性问题。Gate 通过。**

## Project Structure

### Documentation (this feature)

```text
specs/001-kb-core-recall-engine/
├── plan.md              # 本文件
├── research.md          # Phase 0 输出
├── data-model.md        # Phase 1 输出
├── quickstart.md        # Phase 1 输出
├── contracts/           # Phase 1 输出
│   └── node-api.md
└── tasks.md             # /speckit-tasks 输出
```

### Source Code (repository root)

```text
kb-core/                       # 项目根（当前工作目录）
├── Cargo.toml                 # Rust workspace / napi-rs crate
├── package.json               # npm 包入口，scripts: build/test/postinstall
├── build.rs                   # napi-rs 构建脚本
├── index.js                   # JS 入口：加载 .node + 导出 TS 类型
├── index.d.ts                 # 公开 TypeScript 类型声明
│
├── src/                       # Rust 源码
│   ├── lib.rs                 # napi-rs 导出入口
│   ├── config.rs              # KBConfig / InferenceConfig / SystemConfig / ProcessingConfig
│   ├── db/
│   │   ├── mod.rs
│   │   ├── migrations.rs      # SQLite schema 迁移（001~005）
│   │   └── schema.rs          # 表定义常量
│   ├── tantivy_idx/
│   │   ├── mod.rs
│   │   ├── schema.rs          # chunk 级 tantivy schema（已有，13/13）
│   │   └── writer.rs          # commit 批量调度（已有）
│   ├── pipeline/
│   │   ├── mod.rs
│   │   ├── parse.rs           # 解析队列：分流 + 本地/远程 + okf落库 + 派生线性文本
│   │   └── embed.rs           # embedding 队列：drain pending → 推理 → 写vec → 状态流转
│   ├── parse/
│   │   ├── mod.rs
│   │   ├── local.rs           # 本地 extractor（md/txt/eml/code/pdf文本层/docx）
│   │   └── remote.rs          # 远程解析 HTTP 客户端 + 三态熔断
│   ├── embed/
│   │   ├── mod.rs
│   │   └── e5.rs              # fastembed 封装：query:/passage: 前缀注入，intra_threads 约束
│   ├── search/
│   │   ├── mod.rs
│   │   ├── bm25.rs            # tantivy 查询（syntax: text/fielded/raw）
│   │   ├── vector.rs          # sqlite-vec 全量暴力扫描
│   │   ├── rrf.rs             # RRF 融合 + matchedBy 保留
│   │   └── meta.rs            # chunk→block 反查，补 pageRange/bbox/blockTypes/fromImage
│   ├── lock.rs                # flock advisory lock（fs4），NFS fallback
│   ├── tempfile.rs            # 安全临时文件（tempfile crate + ACL on Windows）
│   └── thread_pool.rs         # 三线程池约束 + demote_current_thread()
│
├── scripts/
│   └── download-model.js      # postinstall: 下载 multilingual-e5-small int8 ONNX 到 models/
│
├── models/                    # 模型存放目录（.gitignore，postinstall 填充）
│   └── .gitkeep
│
├── tests/
│   ├── rust/                  # cargo test（单元 + 集成）
│   │   ├── test_pipeline.rs   # async_bm25_before_vector 等新钉住用例
│   │   ├── test_search.rs     # cjk_english_mixed_recall
│   │   └── test_meta.rs       # chunk_to_block_offset_lookup
│   └── node/                  # Vitest Node 集成测试（跑 .node 产物）
│       ├── add-and-search.test.ts
│       ├── status.test.ts
│       └── model-migration.test.ts
│
└── .github/
    └── workflows/
        ├── ci.yml             # PR：cargo test + vitest
        └── publish.yml        # tag v*：跨平台构建 + npm publish
```

**Structure Decision**: 单 Rust crate（napi-rs），JavaScript 层仅作加载器和类型声明。`scripts/download-model.js` 在 `npm install` 后自动拉取模型，放入 `models/` 目录（.gitignore）。CI 与发布分两个 workflow 文件。

## Complexity Tracking

> 无 Constitution Check 违规，此表不填。
