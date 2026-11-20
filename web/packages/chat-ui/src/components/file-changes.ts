import type { Component } from 'vue'

export type ChatUiFileChangeStatusKind = 'modified' | 'added' | 'deleted'

export interface ChatUiFileChangeRow {
  path: string
  absolutePath?: string
  scope: 'workspace' | 'external'
  reportedOnly: boolean
  statusKind?: ChatUiFileChangeStatusKind
  insertions?: number
  deletions?: number
}

export interface ChatUiFileChangeLabels {
  filesChangedUndone: string
  fileChangesCount(count: number): string
  binary: string
  undo: string
  reapply: string
  openWithDefault: string
  revealInFolder: string
  copyPath: string
  copyRelativePath: string
}

export interface ChatUiFileChangeIcons {
  chevronDown: Component
  chevronRight: Component
  rotateCcw: Component
  redo2: Component
  loader2: Component
  externalLink: Component
  folderOpen: Component
  copy: Component
}
