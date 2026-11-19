import type { Component } from 'vue'
import type { ChatUiGeneratedImageBlock } from '../timeline/generated-image-results'

/** Minimal tool block the CUA media panel reads. */
export type ChatUiToolMediaPanelBlock = ChatUiGeneratedImageBlock

export interface ChatUiToolMediaPanelLabels {
  input: string
  output: string
}

/**
 * Host image card (`GeneratedImageResultCard` in desktop). The panel renders it
 * with compact thumbnails and actions disabled, so the host keeps preview I/O
 * and open/save behavior.
 */
export type ChatUiToolMediaImageCard = Component
