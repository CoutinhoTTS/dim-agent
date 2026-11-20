import type { Component } from 'vue'
import type { ChatUiTimelineCopyLabels } from '../timeline/assistant-actions'

export interface ChatUiAssistantActionLabels {
  copyEntireMessage: string
  forkFromHere: string
  retry: string
}

export interface ChatUiAssistantActionCopyLabels extends ChatUiTimelineCopyLabels {
  copySuccess?: string
}

export interface ChatUiAssistantActionIcons {
  check: Component
  copy: Component
  fork: Component
  retry: Component
}
