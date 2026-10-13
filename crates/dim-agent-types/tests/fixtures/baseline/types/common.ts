export type JsonPrimitive = string | number | boolean | null
export type JsonValue = JsonPrimitive | JsonObject | JsonArray
export type JsonObject = { [key: string]: JsonValue | undefined }
export type JsonArray = JsonValue[]

export type JSONSchema = {
  type: "object" | "array" | "string" | "number" | "boolean" | "null"
  properties?: Record<string, JSONSchema>
  items?: JSONSchema
  required?: string[]
  enum?: unknown[]
  description?: string
  [key: string]: unknown
}

export type BackgroundTaskInfo = {
  taskId: string
  wakePolicy: "auto" | "manual"
  outputPath?: string
  description?: string
}

export type BackgroundTaskSummary = {
  taskId: string
  toolName: string
  label?: string
  status: "running" | "completed" | "failed" | "cancelled"
  wakePolicy: "auto" | "manual"
  outputPath?: string
  startedAt: string
  completedAt?: string
}
