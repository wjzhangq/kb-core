# Implementation Plan: Fix High/Medium Severity Bugs from Code Review

**Branch**: `002-fix-review-bugs` | **Date**: 2026-07-18 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/002-fix-review-bugs/spec.md`

## 摘要

修复代码审查发现的7项 HIGH/MEDIUM 级别缺陷，重点消除三处必现 panic（CJK 字节切片、lin_blocks 越界、parse 失败状态不落库），并修复 BM25 静默全表扫描、IndexReader 资源泄漏、embed 失败状态不可见、持锁 await 死锁风险。全部改动限定在 Rust 核心层（`src/pipeline/`、`src/search/`、`src/tantivy_idx/`）及对应类型定义，不新增依赖、不改变外部接口语义。

## Technical Context

**Language/Version**: Rust 2021 edition，rust-version = 1.78

**Primary Dependencies**: napi 2（napi-rs FFI）、tantivy 0.22、rusqlite 0.31（bundled SQLite）、tokio 1（async runtime）、rayon 1（并行）、fastembed 4（ONNX 推理）

**Storage**: SQLite（rusqlite bundled），单写多读，通过 `tokio::sync::Mutex<DbConn>` 序列化写入

**Testing**: `cargo test`（Rust 单元 + 集成测试在 `tests/rust/`）；`vitest run`（Node 集成测试在 `tests/node/`）

**Target Platform**: Linux / macOS / Windows，作为 Node.js native addon 分发

**Project Type**: 原生库（napi-rs native addon）

**Performance Goals**: 修复后搜索延迟不应有可感知退步；IndexReader 改为单例后 segment-reload 延迟 ≤ 现有 ManualReload 策略

**Constraints**: 改动面最小化；不新增 crate 依赖；不破坏现有公开接口（Node API 签名）；所有改动必须通过现有测试套件

**Scale/Scope**: 影响 5 个源文件，约 150–200 行代码改动

## Constitution Check

*GATE: 进入 Phase 0 前评估；Phase 1 设计后复查*

### 原则 I — 范围纪律（Scope Discipline）

**评估**: ✅ PASS
- 7条 FR 全部直接映射到代码审查发现的缺陷，每条改动均可追溯到 spec 中的具体 FR
- LOW 级别问题（token 估算精度、eml CRLF、docx 实体解码、DB transmute UB、Windows DACL）**未纳入**本次范围，符合 spec Assumptions 的明确边界
- 不新增抽象、依赖或接口

**Gate（可测）**: diff 中每个修改文件的每条变更均对应 FR-001 ～ FR-007 之一；存在无法追溯条目即退回。

### 原则 II — 主动澄清（Clarify Before Building）

**评估**: ✅ PASS
- 所有方向性决策已在 spec 中明确：IndexReader 改单例、BM25 解析失败返回空结果、持锁 await 重构由开发者决定具体方式
- spec Assumptions 已记录 `embed_failed` 新状态和 `index.d.ts` 同步更新的决策
- 无需额外澄清，可进入计划阶段

**Gate（可测）**: spec 中无遗留 NEEDS CLARIFICATION；已实现项均有 spec 追溯。

## Project Structure

### Documentation (this feature)

```text
specs/002-fix-review-bugs/
├── plan.md          ← 本文件
├── research.md      ← Phase 0 输出
├── data-model.md    ← Phase 1 输出
├── quickstart.md    ← Phase 1 输出
├── contracts/       ← Phase 1 输出（TS 类型变更）
└── tasks.md         ← /speckit-tasks 输出（待生成）
```

### Source Code (repository root)

```text
src/
├── lib.rs              # FR-007: 消除持锁 await（db lock scope 收窄）
├── pipeline/
│   ├── parse.rs        # FR-001,002,003: chunk_text 修复、lin_blocks 安全映射、失败状态落库
│   └── embed.rs        # FR-006: embed 失败更新 doc status
├── search/
│   ├── bm25.rs         # FR-004: 查询解析失败返回空结果
│   └── mod.rs          # （可能需要随 bm25 调整）
└── tantivy_idx/
    └── mod.rs          # FR-005: IndexReader 单例化

index.d.ts              # 同步新增 embed_failed 状态类型

tests/
├── rust/
│   ├── test_pipeline.rs    # 新增 CJK 切块、空段落、失败状态测试用例
│   └── test_search.rs      # 新增 BM25 格式错误查询测试
└── node/
    └── add-and-search.test.ts  # 现有测试继续通过
```

**Structure Decision**: 单项目布局（无前后端分离），所有改动在 `src/` 内的对应模块，测试跟随模块组织。

## Complexity Tracking

> 无 Constitution Check 违规，此节留空。
