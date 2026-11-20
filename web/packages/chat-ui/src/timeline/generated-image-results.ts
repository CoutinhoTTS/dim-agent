import { normalizeToolName } from './tool-name'

/** Minimal tool block shape the image extractors read. */
export interface ChatUiGeneratedImageBlock {
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
  content?: string
}

export interface GeneratedImageResult {
  id: string
  path: string
  mimeType?: string
  sizeBytes?: number
  revisedPrompt?: string
}

const CUA_VISUAL_TOOL_NAMES = new Set(['get_window_state', 'zoom', 'health_report'])

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null
  }
  return value as Record<string, unknown>
}

const readString = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim().length > 0 ? value.trim() : undefined

const basenameFromPath = (path: string): string =>
  path.split(/[\\/]/).filter(Boolean).at(-1) ?? path

const isImageMimeType = (value: string | undefined): boolean =>
  value?.toLowerCase().startsWith('image/') === true

const addImage = (
  map: Map<string, GeneratedImageResult>,
  input: Omit<GeneratedImageResult, 'id'>,
) => {
  if (!input.path || map.has(input.path)) {
    return
  }
  map.set(input.path, {
    id: input.path,
    path: input.path,
    ...(input.mimeType ? { mimeType: input.mimeType } : null),
    ...(input.sizeBytes !== undefined ? { sizeBytes: input.sizeBytes } : null),
    ...(input.revisedPrompt ? { revisedPrompt: input.revisedPrompt } : null),
  })
}

const readImagesArray = (
  value: unknown,
  map: Map<string, GeneratedImageResult>,
) => {
  if (!Array.isArray(value)) {
    return
  }

  for (const item of value) {
    const record = asRecord(item)
    if (!record) {
      continue
    }
    const path = readString(record.path)
    if (!path) {
      continue
    }
    const mimeType = readString(record.mimeType) ?? readString(record.mediaType) ?? readString(record.mime)
    const sizeBytes = typeof record.sizeBytes === 'number' && Number.isFinite(record.sizeBytes)
      ? record.sizeBytes
      : undefined
    addImage(map, {
      path,
      ...(mimeType ? { mimeType } : null),
      ...(sizeBytes !== undefined ? { sizeBytes } : null),
      ...(readString(record.revisedPrompt) ? { revisedPrompt: readString(record.revisedPrompt) } : null),
    })
  }
}

const readBlobRefsArray = (
  value: unknown,
  map: Map<string, GeneratedImageResult>,
) => {
  if (!Array.isArray(value)) {
    return
  }

  for (const item of value) {
    const record = asRecord(item)
    if (!record) {
      continue
    }
    const path = readString(record.path)
    const mimeType = readString(record.mimeType) ?? readString(record.mediaType) ?? readString(record.mime)
    if (!path || !isImageMimeType(mimeType)) {
      continue
    }
    addImage(map, {
      path,
      ...(mimeType ? { mimeType } : null),
    })
  }
}

const readSingleImageRecord = (
  value: unknown,
  map: Map<string, GeneratedImageResult>,
) => {
  const record = asRecord(value)
  if (!record) return
  const path = readString(record.path)
  const mimeType = readString(record.mimeType) ?? readString(record.mediaType) ?? readString(record.mime) ?? (path ? mimeTypeFromPath(path) : undefined)
  if (!path || !isImageMimeType(mimeType)) {
    return
  }
  addImage(map, {
    path,
    ...(mimeType ? { mimeType } : null),
  })
}

const readMarkdownImageLinks = (
  value: unknown,
  map: Map<string, GeneratedImageResult>,
) => {
  if (typeof value !== 'string') {
    return
  }

  for (const match of value.matchAll(/\[[^\]]+\]\((\/[^)\s]+)\)/g)) {
    const path = match[1]
    if (!path) {
      continue
    }
    addImage(map, {
      path,
      mimeType: mimeTypeFromPath(path),
    })
  }
}

const mimeTypeFromPath = (path: string): string | undefined => {
  const lower = basenameFromPath(path).toLowerCase()
  if (lower.endsWith('.png')) return 'image/png'
  if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) return 'image/jpeg'
  if (lower.endsWith('.webp')) return 'image/webp'
  if (lower.endsWith('.gif')) return 'image/gif'
  return undefined
}

const collectImages = (
  value: unknown,
  map: Map<string, GeneratedImageResult>,
  depth = 0,
) => {
  if (depth > 4) {
    return
  }

  const record = asRecord(value)
  if (!record) {
    readMarkdownImageLinks(value, map)
    return
  }

  readSingleImageRecord(record, map)
  collectImages(record.structuredContent, map, depth + 1)
  readImagesArray(record.images, map)
  readBlobRefsArray(record.blobRefs, map)
  collectImages(record.content, map, depth + 1)
  collectImages(record.output, map, depth + 1)
}

const isBrowserScreenshotBlock = (block: ChatUiGeneratedImageBlock): boolean => {
  if (normalizeToolName(block.toolName) !== 'browser') {
    return false
  }
  const params = asRecord(block.toolParams)
  const action = params && typeof params.action === 'string' ? params.action.trim().toLowerCase() : ''
  return action === 'screenshot'
}

export const extractGeneratedImageResults = (block: ChatUiGeneratedImageBlock): GeneratedImageResult[] => {
  // The browser `screenshot` action stores a PNG blob (BrowserScreenshotRef +
  // blobRefs) that we surface as a thumbnail.
  if (!isBrowserScreenshotBlock(block)) {
    return []
  }

  const map = new Map<string, GeneratedImageResult>()
  collectImages(block.toolResult, map)
  collectImages(block.content, map)
  return [...map.values()]
}

export const extractCuaVisualResults = (block: ChatUiGeneratedImageBlock): GeneratedImageResult[] => {
  if (!CUA_VISUAL_TOOL_NAMES.has(normalizeToolName(block.toolName))) {
    return []
  }

  const map = new Map<string, GeneratedImageResult>()
  collectImages(block.toolResult, map)
  collectImages(block.content, map)
  return [...map.values()]
}
