export type BlobRef = {
  blobId: string
  sessionId: string
  kind: "text" | "binary" | "file" | "stream" | "remote"
  path: string
  ref?: string
  mimeType?: string
  sizeBytes?: number
}

export type BlobMetadata = {
  blobId: string
  sessionId: string
  runId?: string
  messageId?: string
  toolCallId?: string
  mimeType?: string
  displayName?: string
  sizeBytes: number
  createdAt: number
}

export interface BlobManager {
  save(data: Buffer | string, options?: BlobSaveOptions): Promise<BlobRef>
  createTextFile(input: BlobTextFileInput): Promise<BlobRef>
  appendTextFile(blobId: string, text: string): Promise<void>
  finalizeTextFile(blobId: string): Promise<BlobRef>
  read(blobId: string): Promise<Buffer | string>
  delete(blobId: string): Promise<void>
}

export type BlobSaveOptions = {
  kind?: "text" | "binary" | "file" | "stream" | "remote"
  mimeType?: string
  checksum?: string
  filename?: string
}

export type BlobTextFileInput = {
  text: string
  purpose: "tool_result" | "exec_output" | "read_output" | "host_resource"
  toolCallId?: string
  taskId?: string
}
