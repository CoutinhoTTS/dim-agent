import { readdir, readFile } from 'node:fs/promises'
import { dirname, extname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const scanRoots = ['src', 'tests', 'scripts']
const scannedExtensions = new Set(['.ts', '.tsx', '.mjs', '.vue', '.css'])
const selfPath = fileURLToPath(import.meta.url)
const forbiddenImports = [
  { label: "desktop '@/' import", pattern: /(?:from|import|@import)\s*\(?\s*['"]@\// },
  { label: 'window.dimcode', pattern: /window\.dimcode/ },
  { label: 'electron import', pattern: /(?:from|import)\s*\(?\s*['"]electron['"]/ },
  { label: 'electron require', pattern: /require\s*\(\s*['"]electron['"]\s*\)/ },
  { label: 'desktop shared/protocol import', pattern: /(?:from|import)\s*\(?\s*['"]@dimcode\/(?:shared|app-server-protocol|renderer)['"]/ },
  { label: 'desktop store', pattern: /use(?:Chat|Sessions|SessionSend|Account|Appearance)Store/ },
  { label: 'desktop IPC facade', pattern: /getDesktopApi/ },
]

const files = []

async function collect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) {
      await collect(path)
      continue
    }
    if (path !== selfPath && scannedExtensions.has(extname(entry.name))) files.push(path)
  }
}

for (const directory of scanRoots) {
  await collect(join(root, directory))
}

const failures = []
for (const file of files) {
  const source = await readFile(file, 'utf8')
  for (const rule of forbiddenImports) {
    if (rule.pattern.test(source)) {
      failures.push(`${relative(root, file)}: ${rule.label}`)
    }
  }
}

const manifest = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'))
const forbiddenPackages = new Set([
  'electron',
  '@dimcode/shared',
  '@dimcode/app-server-protocol',
  '@dimcode/renderer',
  'pinia',
])
for (const section of ['dependencies', 'peerDependencies', 'devDependencies']) {
  for (const name of Object.keys(manifest[section] ?? {})) {
    if (forbiddenPackages.has(name)) {
      failures.push(`package.json ${section}: ${name}`)
    }
  }
}

if (failures.length > 0) {
  console.error('[chat-ui:boundary] forbidden dependencies found:')
  for (const failure of failures) console.error(`  - ${failure}`)
  process.exit(1)
}

console.log(`[chat-ui:boundary] ${files.length} source, test, and script files checked; no desktop or Electron dependencies found`)
