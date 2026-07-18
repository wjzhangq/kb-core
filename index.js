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
    return require(`@kb-core/${triple}`)
  } catch (e) {
    throw new Error(
      `Failed to load kb-core native binding for ${platform}-${arch}.\n` +
      `Tried: ${localPath} and @kb-core/${triple}\n` +
      `Original error: ${e.message}`
    )
  }
}

module.exports = getBinding()
