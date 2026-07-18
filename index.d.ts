/* eslint-disable */
// Auto-generated TypeScript declarations for kb-core native addon.

export interface KBConfig {
  dataDir: string
  inference?: InferenceConfig
  system?: SystemConfig
  processing?: ProcessingConfig
}

export type InferenceConfig =
  | { mode: 'bm25-only' }
  | {
      mode: 'local-first'
      model?: EmbeddingModelSpec
      modelsDir?: string
      parse?: RemoteParseConfig
    }
  | {
      mode: 'remote'
      model: EmbeddingModelSpec
      embedEndpoint: string
      parse?: RemoteParseConfig
    }

export interface EmbeddingModelSpec {
  name: string
  dim: number
  quantization?: string
}

export interface SystemConfig {
  maxCpuThreads?: number
  lowThreadPriority?: boolean
  tempSecurity?: 'secure-temp' | 'acl-restricted'
}

export interface ProcessingConfig {
  chunkMaxTokens?: number
  chunkOverlapSentences?: number
  embedBatchSize?: number
  parseConcurrency?: number
  readerReloadIntervalMs?: number
  maxFileSizeBytes?: number
  attachmentDenyList?: string[]
}

export interface RemoteParseConfig {
  endpoint: string
  allowRemote?: boolean
  textLayerThreshold?: number
  onRemoteParseUnavailable?: 'wait' | 'text-only' | 'skip'
  timeoutMs?: number
  headers?: Record<string, string>
  breaker?: BreakerConfig
}

export interface BreakerConfig {
  failureThreshold?: number
  resetTimeoutMs?: number
}

export interface AddResult {
  docId: number
  path: string
  status: 'pending_parse' | 'already_indexed'
}

export interface SearchOptions {
  topK?: number
  topN?: number
  rrfK?: number
  aggregate?: 'max' | 'sum' | 'top2sum'
  filter?: SearchFilter
  syntax?: 'text' | 'fielded' | 'raw'
  expandSynonyms?: boolean
  maxCharsPerChunk?: number
  includeText?: boolean
  requireVector?: boolean
  signal?: AbortSignal
}

export interface SearchFilter {
  docType?: string[]
  paths?: string[]
}

export interface SearchResponse {
  results: SearchResult[]
  timing: SearchTiming
  mode: 'bm25-only' | 'bm25+vec'
  vectorCoverage: number
  degraded?: { reason: string }
}

export interface SearchResult {
  docId: number
  path: string
  title?: string
  score: number
  chunks: ChunkResult[]
}

export interface ChunkResult {
  chunkId: number
  text: string
  truncated: boolean
  charOffset: [number, number]
  pageRange?: [number, number]
  bbox?: Array<{ page: number; rect: [number, number, number, number] }>
  blockTypes: string[]
  fromImage: boolean
  matchedBy: Array<'bm25' | 'vector'>
  score: number
}

export interface SearchTiming {
  parseMs: number
  bm25Ms: number
  embedMs: number
  vecMs: number
  rrfMs: number
  aggregateMs: number
  totalMs: number
}

export interface KBStatus {
  total: number
  pendingParse: number
  parsing: number
  parsed: number
  indexed: number
  parseFailed: number
  vectorCoverage: number
  chunkTotal: number
  chunkEmbedDone: number
  chunkEmbedPending: number
  chunkEmbedFailed: number
  walEnabled: boolean
  writerLockHeld: boolean
  modelReady: boolean
  warnings: StatusWarning[]
}

export interface StatusWarning {
  type: 'parse_failed' | 'missing_meta' | 'model_not_found' | 'wal_disabled'
  message: string
  docIds?: number[]
}

export declare class KBLockError extends Error {
  heldBy?: string
}

export declare class KBModelMismatchError extends Error {
  expected: string
  found: string
}

export declare class ModelNotFoundError extends Error {
  modelsDir: string
  modelName: string
}

export declare class KnowledgeBase {
  constructor(options: KBConfig)
  add(path: string | string[]): Promise<AddResult[]>
  search(query: string, options?: SearchOptions): Promise<SearchResponse>
  status(): Promise<KBStatus>
  reindexEmbeddings(model: EmbeddingModelSpec): Promise<void>
  close(): Promise<void>
}
