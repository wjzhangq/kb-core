'use strict'

const { existsSync } = require('fs')
const { join } = require('path')
const { platform, arch } = process

// Published targets: linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc.
// darwin stays here so local dev on macOS can still load a locally-built
// kb-core.darwin-*.node (not published to npm as a sub-package).
const PLATFORM_MAP = {
  darwin: {
    arm64: 'darwin-arm64',
    x64:   'darwin-x64',
  },
  linux: {
    x64:   'linux-x64-gnu',
    arm64: 'linux-arm64-gnu',
  },
  win32: {
    x64: 'win32-x64-msvc',
  },
}

function getBinding() {
  const triple = (PLATFORM_MAP[platform] || {})[arch]
  if (!triple) {
    throw new Error(`Unsupported platform: ${platform}-${arch}`)
  }

  const localPath = join(__dirname, `kb-core.${triple}.node`)
  if (existsSync(localPath)) {
    return require(localPath)
  }

  try {
    return require(`@wjzhangq/kb-core-native-${triple}`)
  } catch (e) {
    throw new Error(
      `Failed to load kb-core native binding for ${platform}-${arch}.\n` +
      `Tried: ${localPath} and @wjzhangq/kb-core-native-${triple}\n` +
      `Original error: ${e.message}`
    )
  }
}

// ── Public error types ──────────────────────────────────────────────────
// The native binding throws `napi::Error` with message strings prefixed by
// the error name (e.g. "KBModelMismatchError: ..."). These classes expose the
// documented public API (node-api.md) so callers can `instanceof`-check and
// read the typed diagnostic fields.

class KBLockError extends Error {
  constructor(message, heldBy) {
    super(message)
    this.name = 'KBLockError'
    if (heldBy !== undefined) this.heldBy = heldBy
  }
}

class KBModelMismatchError extends Error {
  constructor(message, expected, found) {
    super(message)
    this.name = 'KBModelMismatchError'
    if (expected !== undefined) this.expected = expected
    if (found !== undefined) this.found = found
  }
}

class ModelNotFoundError extends Error {
  constructor(message, modelsDir, modelName) {
    super(message)
    this.name = 'ModelNotFoundError'
    if (modelsDir !== undefined) this.modelsDir = modelsDir
    if (modelName !== undefined) this.modelName = modelName
  }
}

const binding = getBinding()

module.exports = Object.assign({}, binding, {
  KBLockError,
  KBModelMismatchError,
  ModelNotFoundError,
})
