import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { KnowledgeBase } from '../../index.js'
import { join } from 'path'
import { mkdirSync, writeFileSync, rmSync } from 'fs'
import os from 'os'

const TEST_DIR = join(os.tmpdir(), `kb-test-status-${Date.now()}`)
const DOC_DIR = join(TEST_DIR, 'docs')

async function waitFor(pred: () => Promise<boolean>, opts = { timeoutMs: 30_000, intervalMs: 300 }) {
  const deadline = Date.now() + opts.timeoutMs
  while (Date.now() < deadline) {
    if (await pred()) return
    await new Promise(r => setTimeout(r, opts.intervalMs))
  }
  throw new Error('waitFor timed out')
}

let kb: InstanceType<typeof KnowledgeBase>

beforeAll(async () => {
  mkdirSync(DOC_DIR, { recursive: true })
  writeFileSync(join(DOC_DIR, 'doc.md'), '# Test\n\nHello world.\n')
  kb = new KnowledgeBase({
    dataDir: TEST_DIR,
    inference: { mode: 'bm25-only' },
    system: { maxCpuThreads: 2 },
  })
})

afterAll(async () => {
  await kb?.close()
  rmSync(TEST_DIR, { recursive: true, force: true })
})

describe('status()', () => {
  it('returns correct structure', async () => {
    const s = await kb.status()
    expect(typeof s.total).toBe('number')
    expect(typeof s.pendingParse).toBe('number')
    expect(typeof s.parsing).toBe('number')
    expect(typeof s.parsed).toBe('number')
    expect(typeof s.indexed).toBe('number')
    expect(typeof s.parseFailed).toBe('number')
    expect(typeof s.vectorCoverage).toBe('number')
    expect(typeof s.walEnabled).toBe('boolean')
    expect(typeof s.writerLockHeld).toBe('boolean')
    expect(Array.isArray(s.warnings)).toBe(true)
  })

  it('walEnabled is true', async () => {
    const s = await kb.status()
    expect(s.walEnabled).toBe(true)
  })

  it('writerLockHeld is true for the owning instance', async () => {
    const s = await kb.status()
    expect(s.writerLockHeld).toBe(true)
  })

  it('parse_failed warning appears for nonexistent file', async () => {
    await kb.add('/nonexistent/totally-fake-file-xyz.md')
    await waitFor(async () => {
      const s = await kb.status()
      return s.warnings.some(w => w.type === 'parse_failed')
    })
    const s = await kb.status()
    const warn = s.warnings.find(w => w.type === 'parse_failed')
    expect(warn).toBeDefined()
    expect(warn!.docIds).toBeDefined()
    expect(warn!.docIds!.length).toBeGreaterThan(0)
  })

  it('parse progress is monotonically non-decreasing during indexing', async () => {
    await kb.add(join(DOC_DIR, 'doc.md'))
    const counts: number[] = []
    for (let i = 0; i < 5; i++) {
      const s = await kb.status()
      counts.push(s.parsed + s.indexed)
      await new Promise(r => setTimeout(r, 100))
    }
    // Counts should never decrease
    for (let i = 1; i < counts.length; i++) {
      expect(counts[i]).toBeGreaterThanOrEqual(counts[i - 1])
    }
  })
})
