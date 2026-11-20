import type { Component } from 'vue'

export interface ChatUiArtifactEntry {
  path: string
  absolutePath?: string
  exists: boolean
  name: string
  canOpen?: boolean
}

export interface ChatUiArtifactLabels {
  count(count: number): string
  allArtifacts(count: number): string
  deleted: string
  previewUnavailable: string
  open: string
  download: string
  showLess: string
  commitOrPush: string
  commit: string
  commitAndPush: string
  push: string
  openWithDefault: string
  revealInFolder: string
}

export interface ChatUiArtifactIcons {
  chevronDown: Component
  chevronRight: Component
  cloudUpload: Component
  download: Component
  externalLink: Component
  folderOpen: Component
  gitCommit: Component
}
