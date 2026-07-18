#!/usr/bin/env node
'use strict'

// Skips download if KB_SKIP_MODEL_DOWNLOAD is set (CI without network).
if (process.env.KB_SKIP_MODEL_DOWNLOAD) {
    console.log('[kb-core] skipping model download (KB_SKIP_MODEL_DOWNLOAD set)')
    process.exit(0)
}

const https = require('https')
const http = require('http')
const fs = require('fs')
const path = require('path')
const crypto = require('crypto')

const MODEL_DIR = path.join(__dirname, '..', 'models', 'multilingual-e5-small')
const BASE_URL = process.env.KB_MODELS_MIRROR ||
    'https://huggingface.co/Xenova/multilingual-e5-small/resolve/main'

const FILES = [
    {
        name: 'model_quantized.onnx',
        url: `${BASE_URL}/onnx/model_quantized.onnx`,
        sha256: null, // populate after first download validation
    },
    {
        name: 'tokenizer.json',
        url: `${BASE_URL}/tokenizer.json`,
        sha256: null,
    },
    {
        name: 'tokenizer_config.json',
        url: `${BASE_URL}/tokenizer_config.json`,
        sha256: null,
    },
    {
        name: 'special_tokens_map.json',
        url: `${BASE_URL}/special_tokens_map.json`,
        sha256: null,
    },
]

async function main() {
    fs.mkdirSync(MODEL_DIR, { recursive: true })

    for (const file of FILES) {
        const dest = path.join(MODEL_DIR, file.name)
        if (fs.existsSync(dest)) {
            console.log(`[kb-core] ${file.name} already present, skipping`)
            continue
        }
        console.log(`[kb-core] downloading ${file.name}…`)
        try {
            await download(file.url, dest)
            if (file.sha256) {
                const hash = await sha256file(dest)
                if (hash !== file.sha256) {
                    fs.unlinkSync(dest)
                    console.warn(`[kb-core] SHA256 mismatch for ${file.name}, retrying once…`)
                    await download(file.url, dest)
                    const hash2 = await sha256file(dest)
                    if (hash2 !== file.sha256) {
                        fs.unlinkSync(dest)
                        throw new Error(`SHA256 mismatch after retry for ${file.name}`)
                    }
                }
            }
            console.log(`[kb-core] ${file.name} downloaded`)
        } catch (e) {
            console.warn(`[kb-core] WARNING: failed to download ${file.name}: ${e.message}`)
            console.warn('[kb-core] You can manually place the model files in:', MODEL_DIR)
            console.warn('[kb-core] Or set KB_MODELS_DIR env var to point to your model directory')
        }
    }
}

function download(url, dest) {
    return new Promise((resolve, reject) => {
        const tmp = dest + '.tmp'
        const file = fs.createWriteStream(tmp)
        const proto = url.startsWith('https') ? https : http

        const request = (u) => {
            proto.get(u, (res) => {
                if (res.statusCode === 301 || res.statusCode === 302) {
                    file.close()
                    return request(res.headers.location)
                }
                if (res.statusCode !== 200) {
                    file.close()
                    fs.unlinkSync(tmp)
                    return reject(new Error(`HTTP ${res.statusCode} for ${u}`))
                }
                res.pipe(file)
                file.on('finish', () => {
                    file.close(() => {
                        fs.renameSync(tmp, dest)
                        resolve()
                    })
                })
            }).on('error', (e) => {
                file.close()
                if (fs.existsSync(tmp)) fs.unlinkSync(tmp)
                reject(e)
            })
        }

        request(url)
    })
}

function sha256file(filePath) {
    return new Promise((resolve, reject) => {
        const hash = crypto.createHash('sha256')
        const stream = fs.createReadStream(filePath)
        stream.on('data', d => hash.update(d))
        stream.on('end', () => resolve(hash.digest('hex')))
        stream.on('error', reject)
    })
}

main().catch((e) => {
    console.warn('[kb-core] postinstall warning:', e.message)
    process.exit(0) // never fail the install
})
