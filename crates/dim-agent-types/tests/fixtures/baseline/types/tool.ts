import type { Logger } from "@archships/dim-log"
import type { ChildProcessByStdio } from "node:child_process"
import type { Readable } from "node:stream"
import type { ContentBlock, Message } from "./content"
import type { BlobManager, BlobRef } from "./blob"
import type { BackgroundTaskInfo, JsonObject, JSONSchema } from "./common"

export type HostEnv = Record<string, string | undefined>

export type ExecShellKind = "posix" | "powershell" | "cmd"

export type ExecShellEdition =
  | "bash"
  | "zsh"
  | "sh"
  | "dash"
  | "ksh"
  | "pwsh"
  | "windows-powershell"
  | "cmd"
  | "git-bash"

export type ExecShellRuntime = {
  readonly executablePath: string
  readonly kind: ExecShellKind
  readonly edition: ExecShellEdition
  readonly outputEncoding: "utf8" | "unknown"
}

export type DetectedExecShell = ExecShellRuntime & {
  readonly displayName: string
  readonly isLoginDefault: boolean
  readonly isAutoTarget: boolean
}

export type ExecShellDetectionResult = {
  readonly shells: readonly DetectedExecShell[]
  readonly autoTarget?: DetectedExecShell
  readonly unsupportedLoginShell?: string
  readonly failedCandidateCount: number
}

export type ExecShellPreference = {
  readonly executablePath?: string
}

export type ExecShellPreferenceResolver = () => Promise<ExecShellPreference>

export type TextFileEncoding =
  | { readonly kind: "utf8" }
  | { readonly kind: "utf16le" }
  | { readonly kind: "utf16be" }
  | { readonly kind: "legacy"; readonly name: string }

export type TextFileBinding = {
  readonly rawRevision: string
  readonly encoding: TextFileEncoding
  readonly encodingSource: "bom" | "workspace" | "user" | "utf8-policy"
  readonly bom: "none" | "utf8" | "utf16le" | "utf16be"
  readonly eol: "none" | "lf" | "crlf" | "mixed"
}

export interface TextFileBindingStore {
  get(resolvedPath: string): TextFileBinding | undefined
  set(resolvedPath: string, binding: TextFileBinding): void
  delete(resolvedPath: string): void
  clear(): void
}

export type ToolSandboxAccess = "read" | "write" | "execute"

export type ToolSandboxSpawnOptions = {
  cwd?: string
  env?: HostEnv
  signal?: AbortSignal
}

export type ToolSandboxChildProcess = ChildProcessByStdio<null, Readable, Readable>

export type ToolSandboxTemporaryScope = {
  readableRoots?: string[]
  writableRoots?: string[]
  allowedDomains?: string[]
}

/**
 * Narrow process sandbox contract consumed by agent tools.
 * Platform-specific isolation details stay behind this interface.
 */
export interface ToolSandboxRuntime {
  assertPathAccess(targetPath: string, access: ToolSandboxAccess): string
  assertDomainAccess?(hostname: string): string
  spawn(
    command: string[],
    options?: ToolSandboxSpawnOptions,
  ): ToolSandboxChildProcess | Promise<ToolSandboxChildProcess>
  withTemporaryScope?(scope: ToolSandboxTemporaryScope): ToolSandboxRuntime
}

export type ToolSandboxRuntimeFactoryInput = {
  sessionId: string
  workspaceRoot: string
  hostDataDir: string
  env: HostEnv
}

export interface ToolSandboxRuntimeFactory {
  create(input: ToolSandboxRuntimeFactoryInput): ToolSandboxRuntime | undefined
}

export type ToolFilePathSelector = "path" | "affectedPaths"

export type ToolFileChangeDeclaration = {
  preExecutePaths?: {
    selector: ToolFilePathSelector
  }
  postExecutePaths?: ToolFilePathSelector
}

export type ToolDefinition = {
  name: string
  description: string
  inputSchema: JSONSchema
  source: ToolSource
  risk?: string
  metadata?: ToolMetadata
  fileChanges?: ToolFileChangeDeclaration
}

export type ToolSource = "static" | "builtin" | "extension" | "dynamic"

export type ToolSourceInfo = {
  kind: "mcp"
  pluginId?: string
  mcpServerId: string
  mcpToolName: string
  /**
   * MCP App preferred display mode declared by the server tool metadata.
   * Missing means the MCP App has no explicit inline surface and behaves as
   * `"fullscreen"` (no visible timeline block). The renderer uses this to
   * decide whether an MCP tool participates in timeline work-block grouping:
   * only explicit `"inline"` apps stay outside groups.
   */
  displayMode?: "inline" | "fullscreen"
}

export type ToolMetadata = {
  version?: string
  author?: string
  docsUrl?: string
  sourcePluginId?: string
  mcpServerId?: string
  mcpOriginalToolName?: string
  [key: string]: unknown
}

export type ToolContext = {
  sessionId: string
  runId: string
  toolCallId: string
  messageId: string
  blobManager: BlobManager
  cwd: string
  logger: Logger
  hostDataDir: string
  env: HostEnv
  execShellRuntime?: ExecShellRuntime
  textFileBindings?: TextFileBindingStore
  abortSignal?: AbortSignal
  sandbox?: ToolSandboxRuntime
  metadata?: Record<string, unknown>
}

export type ToolCall = {
  id: string
  name: string
  input: JsonObject
}

export type ToolResult = {
  content: string | ContentBlock[]
  structuredContent?: JsonObject
  isError?: boolean
  blobRefs?: BlobRef[]
  backgroundTask?: BackgroundTaskInfo
  contextMessages?: Message[]
}

export type ToolExecutor = (
  input: JsonObject,
  context: ToolContext,
) => Promise<ToolResult>
