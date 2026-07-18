<!--
Sync Impact Report
==================
Version change:   [blank template] → 1.0.0 (initial ratification)
Modified principles: N/A (first population)
Added sections:
  - I. 范围纪律 · Scope Discipline
  - II. 主动澄清 · Clarify Before Building
  - Governance
Removed sections: N/A
Templates updated:
  - .specify/templates/plan-template.md  ✅ Constitution Check section already references gates generically
  - .specify/templates/spec-template.md  ✅ No principle-specific references requiring update
  - .specify/templates/tasks-template.md ✅ No principle-specific references requiring update
  - .specify/templates/commands/         ✅ No commands directory exists
Follow-up TODOs:
  - RATIFICATION_DATE set to 2026-07-18 (first population date, treat as ratification date)
-->

# kb-core Constitution

## Core Principles

### I. 范围纪律 · Scope Discipline

只做被明确要求的事，不多做。

- 交付物 MUST 严格对应当前 spec / 请求；每一处新增的功能、文件、抽象、依赖或接口，MUST 能指回 spec 中的明确条目。指不回去的，即为越界。
- 改动面 MUST 最小化：能改一处不改两处；能不新增依赖 / 文件 / 接口就不新增。
- 发现相邻改进点时，MUST 记录并交回 owner 决策，MUST NOT 直接动手。
- MUST NOT 进行未经请求的重构、"顺手优化"，或为将来才可能用到的场景预留扩展点。
- MUST NOT 以"完整性"为名补齐用户没有要求的部分。

**Rationale（kb-core 的既往判例）.** 本项目一贯以"不为不存在的需求预付成本"为准：binary 二段召回不实现、只保留 VectorSearcher trait 边界；answer() / rerank 因超出召回引擎定位而移除而非保留；okf 在无图片语料时退化为单块而非强上版面分析。这些都是本条原则的体现——范围随已确认的定位收敛，而非随想象扩张。

**Gate（可测）.** 变更评审时逐条核对 diff：每个新增的文件 / 依赖 / 公开接口是否对应 spec 的明确要求？存在无法追溯到 spec 的新增项 → 该变更不通过，退回澄清或缩减。

### II. 主动澄清 · Clarify Before Building

需求不明确时，先澄清，不猜。

- 遇到歧义、缺失约束、或存在多种合理解读时，MUST 先提出有针对性的问题，MUST NOT 选一个默认值闷头实现。
- 方向性决策（如：更换默认模型、改召回策略、约定数据格式、确定对外消费面）MUST 在动工前向 owner 澄清并留下决策记录。
- 澄清 SHOULD 一次问到点子上：提出最少、最能消除分歧的问题（通常 1–3 个），并尽量给出可选项以便回答。
- 已能从上下文确定的，MUST NOT 明知故问；真正影响方向的，MUST 问。
- MUST NOT 用一堆假设填补空白后交付一个方向就错了的结果；MUST NOT 把"我以为你想要"当成"你要求的"。

**Rationale（kb-core 的既往判例）.** 本项目的每次重大转向——万级 vs 百万级规模定位、中文为主 vs 中英混排选型、search 框 vs 问答框形态、解析出网的隐私权衡——都是先澄清、由 owner 拍板后才落方案的。方向猜错的返工成本，远高于问一句的成本。

**Gate（可测）.** spec 中若存在方向性未定项，MUST 有对应的澄清问题与决策记录，否则不得进入 /plan。评审时检查：本次工作是否清楚"做成什么样算对"？不清楚而仍已实现 → 违规。

## Governance

- **本 constitution 优先。** 与其他约定、习惯或临时指令冲突时，以本文件为准。当 Principle I 与 Principle II 同时适用而张力出现时：澄清优先于猜测，克制优先于自作主张——拿不准就少做、先问。
- **修订流程.** 任何修订 MUST 经项目 owner 批准；MUST 同步更新版本号、Last Amended 日期与顶部 Sync Impact Report；并按 spec-kit 流程传播到依赖模板（plan / spec / tasks / commands）。
- **版本策略（语义化 MAJOR.MINOR.PATCH）.**
  - MAJOR：向后不兼容的治理 / 原则移除或重定义。
  - MINOR：新增原则或章节，或对既有指引的实质性扩充。
  - PATCH：措辞澄清、错字修正、非语义性细化。
- **合规审查.** 每个 /plan 与 /analyze 阶段 MUST 执行 Constitution Check：凡触碰两条原则的变更，MUST 先通过对应 Gate（§Core Principles），否则先澄清或缩减范围，再继续。

**Version**: 1.0.0 | **Ratified**: 2026-07-18 | **Last Amended**: 2026-07-18
