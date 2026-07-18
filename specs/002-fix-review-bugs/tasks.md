# Tasks: Fix High/Medium Severity Bugs from Code Review

**Input**: Design documents from `specs/002-fix-review-bugs/`

**Prerequisites**: [plan.md](./plan.md) · [spec.md](./spec.md) · [research.md](./research.md) · [data-model.md](./data-model.md) · [contracts/](./contracts/)

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no open dependencies)
- **[Story]**: Maps to user story in spec.md (US1–US7)
- Exact file paths included in every task

## Path Conventions

Single project layout. All Rust source under `src/`, tests under `tests/`.

---

## Phase 1: Setup

**Purpose**: Verify working baseline and create test fixtures before any fix lands.

- [X] T001 Verify baseline passes: `cargo test` and `KB_SKIP_MODEL_DOWNLOAD=1 npm test` both green (document any pre-existing failures)
- [X] T002 [P] Add CJK test fixture string (200+ Chinese characters, default chunk boundary lands inside a Han character) as a const in `tests/rust/test_pipeline.rs`
- [X] T003 [P] Add non-contiguous block_id fixture (block_ids: [0, 2, 5]) as a helper in `tests/rust/test_pipeline.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: No shared blocking prerequisite exists for this bug-fix feature — each FR targets an independent file. Phase 1 completion unblocks all user story phases.

**⚠️ CRITICAL**: Complete Phase 1 before starting any user story phase.

---

## Phase 3: User Stories 1–3 — Parse Engine Fixes (Priority: P1) 🎯 MVP

All three P1 stories modify `src/pipeline/parse.rs`. They must be applied sequentially within this file; the tests can be written in parallel with each other.

**Goal**: Eliminate all three panic-inducing defects in the parse pipeline so CJK documents, documents with empty/non-contiguous paragraphs, and parse failures all behave correctly.

**Independent Test**: `cargo test test_chunk_cjk test_empty_paragraph test_parse_failed_status -- --nocapture` all pass; adding a real Chinese Markdown file via the Node integration test completes without process crash.

### Implementation — User Story 1 (FR-001: CJK byte-safe chunking)

- [X] T004 [US1] In `chunk_text` (`src/pipeline/parse.rs`): replace raw byte-index `end` with `text.floor_char_boundary((start + max_chars).min(text.len()))` to guarantee valid UTF-8 slice boundaries (see research.md §1)
- [X] T005 [US1] In `chunk_text` (`src/pipeline/parse.rs`): ensure `actual_end` from `rfind` fallbacks also passes through `floor_char_boundary` before the final `text[start..actual_end]` slice

### Implementation — User Story 2 (FR-002: lin_blocks safe mapping)

- [X] T006 [US2] Change `build_linear_text` return type from `Vec<(i64, i64)>` to `HashMap<u32, (i64, i64)>` in `src/pipeline/parse.rs`, keying by `block.block_id` (see research.md §2)
- [X] T007 [US2] Update the block-insert loop in `process_doc` (`src/pipeline/parse.rs`): replace `lin_blocks[b.block_id as usize]` with `lin_blocks.get(&b.block_id)`, skip + warn on missing key

### Implementation — User Story 3 (FR-003: parse failure status guarantee)

- [X] T008 [US3] Add best-effort `parse_failed` fallback in the spawner `Err` branch (`src/pipeline/parse.rs:35–39`): after logging, acquire db lock and execute `UPDATE documents SET status='parse_failed' WHERE doc_id=?1 AND status='parsing'`; swallow the result with `let _ = ...` (see research.md §3)

### Tests — US1/US2/US3

- [X] T009 [P] [US1] Add `test_chunk_cjk` in `tests/rust/test_pipeline.rs`: input = CJK fixture from T002, assert all chunks are valid UTF-8 and no panic
- [X] T010 [P] [US2] Add `test_empty_paragraph_blocks` in `tests/rust/test_pipeline.rs`: input = non-contiguous block_id fixture from T003, assert `build_linear_text` returns correct HashMap, assert block-insert loop skips missing IDs without panic
- [X] T011 [P] [US3] Add `test_parse_failed_status` in `tests/node/add-and-search.test.ts`: add a non-existent file path, wait 2 s, assert `status().details` shows `parse_failed` (not `parsing`)

**Checkpoint**: `cargo test` and `KB_SKIP_MODEL_DOWNLOAD=1 npm test` pass. Adding a Chinese `.md` file via Node API completes without crash.

---

## Phase 4: User Story 4 — BM25 Invalid Query (Priority: P2)

**Goal**: Format-invalid query strings return empty results instead of silently scanning the entire corpus.

**Independent Test**: `cargo test test_bm25_invalid_query -- --nocapture` passes; result count is 0 for a malformed query against a non-empty index.

- [X] T012 [US4] In `bm25_search` (`src/search/bm25.rs:30–34`): replace `unwrap_or_else(|_| Box::new(AllQuery))` with `match`; on `Err(_)` return `Ok(vec![])` immediately (see research.md §4)
- [X] T013 [US4] Add `test_bm25_invalid_query` in `tests/rust/test_search.rs`: index 3 docs, call BM25 with `"[unclosed"`, assert `result.len() == 0`

**Checkpoint**: Invalid queries return empty, not full-corpus results.

---

## Phase 5: User Story 5 — IndexReader Singleton (Priority: P2)

**Goal**: `IndexReader` is created once per `TantivyIndex` instance and reused, eliminating per-search resource leaks.

**Independent Test**: 1000 consecutive searches do not increase process RSS or file-handle count measurably (see quickstart.md Scenario 5).

- [X] T014 [US5] Add `reader: IndexReader` field to `TantivyIndex` struct in `src/tantivy_idx/mod.rs`
- [X] T015 [US5] In `TantivyIndex::open_or_create` (`src/tantivy_idx/mod.rs`): build the `IndexReader` with `ReloadPolicy::Manual` and store it in the struct
- [X] T016 [US5] Add `pub fn reader(&self) -> &IndexReader` getter (or expose field as `pub`) in `src/tantivy_idx/mod.rs`; remove the existing `reader()` method that creates a new instance each call
- [X] T017 [US5] Update `bm25_search` in `src/search/bm25.rs` to call `tantivy.reader()` (now returns `&IndexReader`) instead of constructing a new reader; remove the `let reader = tantivy.reader()?` + `reader.reload()?` pattern and replace with `tantivy.reader().reload()?`

**Checkpoint**: Compile succeeds; `cargo test` still passes; no per-search `IndexReader` construction.

---

## Phase 6: User Story 6 — Embed Failure Status (Priority: P2)

**Goal**: When a vector embedding batch fails, the affected document's status is updated to `embed_failed` so callers can observe the failure via `status()`.

**Independent Test**: With an intentionally broken model path, `status()` returns `embed_failed` for the affected document while BM25 search still returns results.

- [X] T018 [US6] In the embed batch `Err` branch (`src/pipeline/embed.rs:62–72`): after marking chunks `embed_status=2`, also execute `UPDATE documents SET status='embed_failed', updated_at=?1 WHERE doc_id=(SELECT doc_id FROM chunks WHERE chunk_id=?2)` for each affected chunk_id
- [X] T019 [US6] Add `'embed_failed'` to the `DocumentStatus` union in `index.d.ts` (see contracts/api-status-types.md)

**Checkpoint**: `index.d.ts` reflects new status; embed failure visible via `status()` API.

---

## Phase 7: User Story 7 — Eliminate Hold-Lock-Await Deadlock (Priority: P2)

**Goal**: The `add()` path releases the DB lock before awaiting the parse channel send, eliminating the theoretical deadlock when the channel is full.

**Independent Test**: Concurrent `add()` + `search()` calls (20 files, 10 searches) complete without hanging (see quickstart.md Scenario 7).

- [X] T020 [US7] In `KnowledgeBase::add` (`src/lib.rs`): restructure the insert block so `doc_id` is extracted inside the lock scope and the `Arc<Mutex>` guard is dropped before `self.parse_tx.send(doc_id).await` (see research.md §7)

**Checkpoint**: Concurrent add+search does not deadlock; `cargo test` still passes.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [X] T021 Run full validation: `cargo test` and `KB_SKIP_MODEL_DOWNLOAD=1 npm test` — all tests green including new ones from T009–T013
- [X] T022 [P] Run quickstart.md Scenario 5 (leak check) and Scenario 7 (deadlock check) manually; confirm expected output — S5 RSS delta 80 KB (<500 KB), S7 completed without deadlock
- [X] T023 [P] **Read-only audit**: verify no other union type in `index.d.ts` references the document status string besides `DocumentStatus`; **do NOT modify** unless a violation directly breaks FR-006 or FR-007 contracts — log any issues found as follow-up rather than fixing in-place
- [X] T024 [P] [US5] Add `test_reader_no_fd_leak` in `tests/rust/test_search.rs`: open the tantivy index, run 1000 BM25 searches, record fd count before/after via `/proc/self/fd` (Linux) or `lsof` (macOS); assert delta ≤ 10. Annotate with `#[cfg(not(target_os = "windows"))]` — SC-005 on Windows verified manually via quickstart.md Scenario 5

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 3 (P1 fixes)**: Depends on Phase 1 completion; T006 must precede T007 (same function); T004 must precede T005 (same function)
- **Phases 4–7 (P2 fixes)**: All depend on Phase 1 completion; can proceed in parallel with each other and with Phase 3 (different files)
- **Phase 8 (Polish)**: Depends on all desired phases complete

### User Story Dependencies

- **US1, US2, US3** (Phase 3): Share `src/pipeline/parse.rs` — must be sequential within the file
- **US4** (Phase 4, `bm25.rs`): Independent — can run alongside Phase 3
- **US5** (Phase 5, `tantivy_idx/mod.rs` + `bm25.rs`): Shares `bm25.rs` with US4 — coordinate or sequence after T012–T013
- **US6** (Phase 6, `embed.rs` + `index.d.ts`): Fully independent
- **US7** (Phase 7, `lib.rs`): Fully independent

### Parallel Opportunities

- T002 and T003 (fixtures) can run in parallel
- T009, T010, T011 (tests) can be written in parallel with each other
- Phases 4, 6, 7 can run in parallel (no shared files)
- Phase 5 must sequence after Phase 4 on the `bm25.rs` file

---

## Parallel Example: Phase 3 (P1 Fixes)

```
Sequential (same file src/pipeline/parse.rs):
  T004 → T005 → T006 → T007 → T008

Parallel (different files, no deps):
  T009 (test_pipeline.rs) || T010 (test_pipeline.rs — different test fn) || T011 (add-and-search.test.ts)
```

## Parallel Example: P2 Phases

```
After Phase 1 completes:
  Phase 4 (bm25.rs: T012–T013)
  Phase 6 (embed.rs + index.d.ts: T018–T019)  ← fully parallel
  Phase 7 (lib.rs: T020)                        ← fully parallel
  Phase 5 (tantivy_idx/mod.rs: T014–T016, then bm25.rs: T017)  ← sequence T017 after Phase 4
```

---

## Implementation Strategy

### MVP First (P1 Panics Only — User Stories 1–3)

1. Complete Phase 1: Setup (T001–T003)
2. Complete Phase 3: US1 + US2 + US3 (T004–T011)
3. **STOP and VALIDATE**: CJK docs and empty-paragraph docs index correctly, parse failures report status
4. Ship — the three must-fix panics are resolved

### Full Fix Delivery

1. Setup (Phase 1) → P1 fixes (Phase 3) → **validate**
2. P2 fixes in parallel: Phases 4, 6, 7 together; Phase 5 after Phase 4
3. Polish (Phase 8) → full regression run

---

## Notes

- [P] = different files or no open dependencies within this phase
- [Story] label maps task to spec.md user story for traceability
- `floor_char_boundary` is stable since Rust 1.65; project requires 1.78 — no feature flags needed
- No new crate dependencies required for any task
- Commit after each phase checkpoint for clean rollback points
