import { readdir, readFile } from 'node:fs/promises'
import { dirname, extname, join, resolve } from 'node:path'
import { gzipSync } from 'node:zlib'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const dist = join(root, 'dist')
const defaultBudget = 900 * 1024
const budget = Number(process.env.CHAT_UI_MAX_GZIP_BYTES ?? defaultBudget)
const webConfigIndex = process.argv.indexOf('--web-config')
const webConfig = webConfigIndex === -1 ? undefined : process.argv[webConfigIndex + 1]

if (!Number.isFinite(budget) || budget <= 0) {
  throw new Error('CHAT_UI_MAX_GZIP_BYTES must be a positive number')
}

if (webConfig !== undefined) {
  const config = await readFile(resolve(webConfig), 'utf8')
  if (!/@terrastruct\\?\/d2/.test(config)) {
    throw new Error(`${webConfig} must keep the @terrastruct/d2 alias; the shared timeline assumes it`)
  }
}

const files = []
async function collect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) {
      await collect(path)
      continue
    }
    if (['.js', '.mjs', '.css'].includes(extname(entry.name))) files.push(path)
  }
}

try {
  await collect(dist)
} catch (error) {
  if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
    throw new Error('[chat-ui:budget] dist is missing; run the package build first')
  }
  throw error
}

if (files.length === 0) {
  throw new Error('[chat-ui:budget] dist has no JS or CSS output')
}

let total = 0
for (const file of files) {
  total += gzipSync(await readFile(file)).byteLength
}

if (total > budget) {
  throw new Error(`[chat-ui:budget] ${(total / 1024).toFixed(1)}KB gzip exceeds the ${(budget / 1024).toFixed(0)}KB budget`)
}

console.log(`[chat-ui:budget] ${(total / 1024).toFixed(1)}KB gzip across ${files.length} files (budget ${(budget / 1024).toFixed(0)}KB)`)
