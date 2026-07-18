# Contract: KnowledgeBase Node.js API — Status Type Changes

**File**: `index.d.ts`
**Change type**: Additive (backward-compatible)
**Introduced by**: FR-006

---

## DocumentStatus

### Before

```typescript
type DocumentStatus =
  | 'pending_parse'
  | 'parsing'
  | 'parsed'
  | 'indexed'
  | 'parse_failed'
  | 'already_indexed'
```

### After

```typescript
type DocumentStatus =
  | 'pending_parse'
  | 'parsing'
  | 'parsed'
  | 'indexed'
  | 'parse_failed'
  | 'embed_failed'      // NEW: vector embedding failed; BM25 search still works
  | 'already_indexed'
```

---

## KBStatus.details (per-document status in status())

The `details` array entries already use `DocumentStatus`. No structural change needed — the new `embed_failed` value flows through automatically once the Rust layer emits it.

---

## Behavioral Contract Changes

| Scenario | Before | After |
|----------|--------|-------|
| Vector embed batch fails | doc stays `parsed` forever | doc transitions to `embed_failed` |
| BM25 query parse error | returns full corpus results | returns empty results array |
| CJK document added | panic / process crash | document indexed normally |
| Document with empty paragraphs | panic / process crash | document indexed normally |
| Parse failure | doc may stay in `parsing` | doc guaranteed to reach `parse_failed` |

---

## Compatibility

- `embed_failed` is a new possible value in an open union — existing consumers that check only specific status strings are unaffected
- All existing `AddResult`, `SearchResponse`, `KBStatus` shapes are unchanged
- No method signatures change
