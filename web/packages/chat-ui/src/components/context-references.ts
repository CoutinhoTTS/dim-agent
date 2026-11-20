import type { Component } from 'vue'

export type ChatUiContextReferenceSource = 'file' | 'document' | 'spreadsheet' | 'conversation' | 'browser'

export interface ChatUiSentReferenceMetadata {
  source: ChatUiContextReferenceSource
  text: string
  selectedText?: string
  comment?: string
}

export interface ChatUiContextReferenceLabels {
  selectedText: string
  userComment: string
  saveAnnotation: string
  editAnnotation: string
  removeAnnotation: string
}

export interface ChatUiContextReferenceActionIcons {
  check: Component
  pencil: Component
  remove: Component
}

export interface ChatUiContextReferenceIcons extends ChatUiContextReferenceActionIcons {
  source: Record<ChatUiContextReferenceSource, Component>
}
