import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { KnowledgeBase } from '../../index.js'
import { join } from 'path'
import { mkdirSync, writeFileSync, rmSync } from 'fs'
import os from 'os'

const TEST_DIR = join(os.tmpdir(), `kb-test-search-${Date.now()}`)
const DOC_DIR = join(TEST_DIR, 'docs')

async function waitFor(
  pred: () => Promise<boolean>,
  opts = { timeoutMs: 60_000, intervalMs: 500 }
) {
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

  // Create test documents with CJK + English mixed content
  writeFileSync(join(DOC_DIR, 'mqtt-guide.md'), `
# MQTT Protocol Guide

MQTT is a lightweight IoT messaging protocol.
BLE (Bluetooth Low Energy) advertising packets carry device identifiers.
MQTT 是一种轻量级的物联网消息传输协议，广泛用于嵌入式设备通信。
BLE 广播包用于设备发现和低功耗通信场景。
`)

  writeFileSync(join(DOC_DIR, 'intro.md'), `
# Introduction

This document introduces the knowledge base recall engine.
这是一个知识库召回引擎的介绍文档，支持中英文混排的语义检索。
`)

  kb = new KnowledgeBase({
    dataDir: TEST_DIR,
    inference: { mode: 'bm25-only' }, // use bm25-only for speed in CI
    system: { maxCpuThreads: 2 },
  })
})

afterAll(async () => {
  await kb?.close()
  rmSync(TEST_DIR, { recursive: true, force: true })
})

describe('add() and BM25 search', () => {
  it('add() returns immediately with pending_parse status', async () => {
    const results = await kb.add([
      join(DOC_DIR, 'mqtt-guide.md'),
      join(DOC_DIR, 'intro.md'),
    ])
    expect(results).toHaveLength(2)
    expect(results[0].status).toMatch(/pending_parse|already_indexed/)
    expect(typeof results[0].docId).toBe('number')
  })

  it('duplicate add() returns already_indexed', async () => {
    const results = await kb.add(join(DOC_DIR, 'mqtt-guide.md'))
    expect(results[0].status).toBe('already_indexed')
  })

  it('BM25 search finds documents after parsing', async () => {
    await waitFor(async () => {
      const s = await kb.status()
      return s.parsed + s.indexed >= 2
    })

    const r = await kb.search('MQTT protocol')
    expect(r.results.length).toBeGreaterThan(0)
    expect(r.mode).toMatch(/bm25/)

    const chunk = r.results[0].chunks[0]
    expect(chunk.matchedBy).toContain('bm25')
    expect(Array.isArray(chunk.charOffset)).toBe(true)
    expect(chunk.charOffset[1]).toBeGreaterThan(chunk.charOffset[0])
    expect(Array.isArray(chunk.blockTypes)).toBe(true)
    expect(chunk.blockTypes.length).toBeGreaterThan(0)
  })

  it('SearchResponse has correct shape (no rerank/LLM fields)', async () => {
    const r = await kb.search('knowledge base')
    expect(r).toHaveProperty('results')
    expect(r).toHaveProperty('timing')
    expect(r).toHaveProperty('mode')
    expect(r).toHaveProperty('vectorCoverage')

    // Must NOT have rerank/LLM fields
    expect(r).not.toHaveProperty('reranked')
    expect(r).not.toHaveProperty('answer')
    expect(r).not.toHaveProperty('llmResponse')

    expect(typeof r.timing.totalMs).toBe('number')
    expect(typeof r.timing.bm25Ms).toBe('number')
  })

  it('charOffset is a valid non-zero interval', async () => {
    const r = await kb.search('introduction')
    if (r.results.length > 0) {
      const chunk = r.results[0].chunks[0]
      expect(chunk.charOffset[1]).toBeGreaterThan(chunk.charOffset[0])
    }
  })

  it('fromImage is false for plain text docs', async () => {
    const r = await kb.search('MQTT')
    if (r.results.length > 0) {
      const chunk = r.results[0].chunks[0]
      expect(chunk.fromImage).toBe(false)
    }
  })
})
