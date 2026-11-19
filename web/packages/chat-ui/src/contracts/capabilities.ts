export interface ChatUiCapabilities {
  /** Commands are writable. A read-only surface still receives the contract. */
  interactive: boolean
  terminal: boolean
  browserPanel: boolean
  filePreview: boolean
  mcpApps: boolean
  notifications: boolean
  attachments: boolean
}
