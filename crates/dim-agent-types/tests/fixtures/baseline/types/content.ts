import type { JsonObject } from "./common"

export type ImageSource = {
  type: "base64" | "url"
  mediaType: string
  data?: string
  url?: string
}

export type TextContentBlock = {
  type: "text"
  text: string
}

export type ImageContentBlock = {
  type: "image"
  source: ImageSource
}

export type ContentBlock = TextContentBlock | ImageContentBlock

export type TextPart = {
  type: "text"
  text: string
  /** ISO 8601 wall-clock time of the first text delta arrival. */
  startTime?: string
  /** ISO 8601 wall-clock time of the last text delta arrival. */
  endTime?: string
}

export type ImagePart = {
  type: "image"
  source: ImageSource
}

export type ToolUsePart = {
  type: "tool_use"
  id: string
  name: string
  input: JsonObject
  /** ISO 8601 wall-clock time of the `tool_use` event arrival. */
  startTime?: string
}

export type ToolResultPart = {
  type: "tool_result"
  tool_use_id: string
  content: string | ContentBlock[]
  structuredContent?: JsonObject
  is_error?: boolean
}

export type ThinkingPart = {
  type: "thinking"
  thinking: string
  /**
   * ISO 8601 wall-clock time of the first `thinking_start` arrival of this
   * thinking segment.
   */
  startTime?: string
  /**
   * ISO 8601 wall-clock time of the matching `thinking_end` arrival (or the
   * message close when the provider stops the stream without one).
   */
  endTime?: string
}

export type MessagePart = TextPart | ImagePart | ToolUsePart | ToolResultPart | ThinkingPart

export type MessageContent = string | MessagePart[]

export type Message = {
  role: "user" | "assistant" | "tool_result"
  content: MessageContent
}

/**
 * Marker text that prefixes a synthetic user message carrying Codex v2 remote
 * compaction credentials (encrypted blobs). The message is invisible to the
 * model: `normalizeCodexBody` in the fetch layer detects this prefix and
 * replaces the entire message with real `{ type: "compaction", ... }` input
 * items before the request hits the network.
 *
 * The second line of the marker message is a JSON payload: `{"items":[...]}`.
 */
export const CODEX_COMPACTION_MARKER_TEXT = "[dimcode:codex-compaction-item:internal]"

/** Maximum serialized bytes of persisted remote compaction items. */
export const COMPACTION_RAW_ITEMS_MAX_BYTES = 64 * 1024
