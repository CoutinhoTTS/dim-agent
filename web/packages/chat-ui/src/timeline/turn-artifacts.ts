/**
 * Per-turn artifact showcase rules. The extension table is presentation
 * policy: it filters changed paths down to deliverable files. It is not part
 * of the agent contract.
 */
export type ArtifactCategory = 'web' | 'image' | 'markdown' | 'document' | 'data' | 'media'

const EXTENSION_CATEGORIES: Record<string, ArtifactCategory> = {
  html: 'web',
  htm: 'web',
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  gif: 'image',
  svg: 'image',
  webp: 'image',
  md: 'markdown',
  docx: 'document',
  docm: 'document',
  dotx: 'document',
  dotm: 'document',
  pptx: 'document',
  pptm: 'document',
  ppsx: 'document',
  ppsm: 'document',
  potx: 'document',
  potm: 'document',
  xlsx: 'document',
  xlsb: 'document',
  xlsm: 'document',
  xltx: 'document',
  xltm: 'document',
  pdf: 'document',
  doc: 'document',
  ppt: 'document',
  xls: 'document',
  csv: 'data',
  mp4: 'media',
  webm: 'media',
  mp3: 'media',
  wav: 'media',
}

export interface ChatUiTimelineArtifactMessage {
  id: string
  metadata?: Readonly<Record<string, unknown>>
}

export interface ChatUiTimelineArtifactTurn {
  userMessage: ChatUiTimelineArtifactMessage | null
}

/** undefined = the file is not a showcased artifact type. */
export function classifyArtifact(path: string): ArtifactCategory | undefined {
  const basename = path.split(/[\\/]/).filter(Boolean).at(-1) ?? ''
  const dotIndex = basename.lastIndexOf('.')
  if (dotIndex <= 0) {
    return undefined
  }
  const extension = basename.slice(dotIndex + 1).toLowerCase()
  return EXTENSION_CATEGORIES[extension]
}

/** Resolve the run id persisted on the turn's opening user message. */
export function resolveTurnRunId(turn: ChatUiTimelineArtifactTurn): string | undefined {
  return resolveMessageRunId(turn.userMessage)
}

export function resolveMessageRunId(
  message: ChatUiTimelineArtifactMessage | null | undefined,
): string | undefined {
  const metadataRunId = message?.metadata?.runId
  if (typeof metadataRunId === 'string' && metadataRunId.trim().length > 0) {
    return metadataRunId.trim()
  }
  return undefined
}
