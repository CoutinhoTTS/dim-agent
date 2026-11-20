import type {
  ChatUiTimelineMessageDetailInput,
  ChatUiTimelineRunChangedPathsInput,
} from '../contracts/timeline'
import type {
  ChatUiRunChangedPathEntry,
  ChatUiTimelineMessageDetail,
  ChatUiTimelineUserMessageMarker,
} from '../timeline/user-message-rail'

export interface ChatUiTimelineUserMessageRailLabels {
  emptyChangedPaths: string
  timeline(count: number): string
  jumpToMessage(preview: string): string
  option(index: number, count: number, preview: string): string
}

/**
 * The user-message rail owns hover/focus/dense-rail state and rendering. The
 * host supplies the marker projection, labels, and the two reads the card
 * needs; it never reads a desktop store or the App Server from here.
 */
export interface ChatUiTimelineUserMessageRailProps {
  sessionId: string | null
  markers: readonly ChatUiTimelineUserMessageMarker[]
  loadedMarkers: readonly ChatUiTimelineUserMessageMarker[]
  scrollParent?: HTMLElement | Window | null
  timelineRoot?: HTMLElement | null
  rowDomSignature?: string
  layoutResizing?: boolean
  labels: ChatUiTimelineUserMessageRailLabels
  readMessageDetail?: (
    input: ChatUiTimelineMessageDetailInput,
  ) => Promise<ChatUiTimelineMessageDetail | null | undefined>
  readRunChangedPaths?: (
    input: ChatUiTimelineRunChangedPathsInput,
  ) => Promise<ChatUiRunChangedPathEntry[] | null>
}
