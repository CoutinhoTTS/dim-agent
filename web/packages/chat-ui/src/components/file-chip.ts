/**
 * Payload emitted when a file chip is activated. Hosts map it onto their file
 * opener (preview panel, system app, or a no-op) and decide how to resolve the
 * relative href; the package never opens files itself.
 */
export interface ChatUiFileChipOpenPayload {
  /** Absolute path, when known (file:// chips). */
  absolutePath?: string
  /** Path relative to the session cwd (Markdown workspace links). */
  relativeHref?: string
  /** Treated as a directory (no file extension). */
  isDirectory?: boolean
  /** Cmd/Ctrl+click: the host should use the system default app. */
  external: boolean
}
