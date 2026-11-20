<script setup lang="ts">
import { computed, type Component } from 'vue'
import { useOptionalChatUiHost } from '../contracts/host'
import type { ChatUiTimelineMessageActionPayload } from '../contracts/timeline'
import type { ChatUiTimelineSearchMatch } from '../timeline/search'
import type { ChatUiTurnTerminalState } from './assistant-message'
import type { ChatUiTimelineSubagentTarget } from './block-group'
import type {
  ChatUiTimelineForkMessage,
  ChatUiTimelinePreviewTarget,
  ChatUiTimelineRow,
  ChatUiTimelineRowBlock,
  ChatUiTimelineRowMessage,
} from './timeline-row'
import TimelineWindowTruncationDivider from './TimelineWindowTruncationDivider.vue'

defineOptions({
  name: 'TimelineRow',
})

// `mcpAppInlineAnchorsEnabled` needs an explicit default: Vue casts an absent
// optional boolean prop to `false`, not `undefined`, so without one the
// interactive timeline (which never passes the flag) would forward a hard
// `false` and defeat the child's own default — killing MCP App registration
// and with it every right-panel widget tab.
const props = withDefaults(defineProps<{
  row: ChatUiTimelineRow
  sessionId: string | null
  isLatestTurn?: boolean
  timelineRunning?: boolean
  turnDetailsExpanded: boolean
  turnDetailsToggleVisible: boolean
  blockExpandedById: Readonly<Record<string, boolean>>
  blockGroupExpandedById: Readonly<Record<string, boolean>>
  searchQuery?: string
  searchMatchesBySource?: Readonly<Record<string, readonly ChatUiTimelineSearchMatch[]>>
  activeSearchMatchId?: string | null
  elapsedNowMs?: number
  markdownViewportPriority?: boolean
  assistantUserMessageId?: string | null
  retryUserMessage?: ChatUiTimelineRowMessage | null
  assistantActionsAlwaysVisible?: boolean
  assistantCompletionTimeOnly?: boolean
  assistantRetryLabel?: string
  mcpAppInlineAnchorsEnabled?: boolean
  readOnly?: boolean
  /** Resolved run id for the latest-turn artifacts / file surfaces. */
  turnRunId?: string | null
  /** Host capability: the file-changes surface is enabled. */
  fileChangesVisible?: boolean
  /** Host answer for "can this session send". */
  canSend?: boolean
  canBranchFromUserMessage?: (message: ChatUiTimelineRowMessage) => boolean
  isRetryableUserMessage?: (message: ChatUiTimelineRowMessage) => boolean
  readTurnTerminal?: (message: ChatUiTimelineRowMessage) => ChatUiTurnTerminalState | undefined
  resolveSystemNoticeText?: (message: ChatUiTimelineRowMessage) => string | undefined
  readSubagentTarget?: (block: ChatUiTimelineRowBlock) => ChatUiTimelineSubagentTarget | null
  userMessageComponent: Component
  assistantMessageComponent: Component
  artifactsComponent: Component
  fileChangesComponent: Component
  assistantActionsComponent: Component
  compressionDividerComponent: Component
}>(), {
  isLatestTurn: false,
  timelineRunning: false,
  searchQuery: '',
  searchMatchesBySource: () => ({}),
  activeSearchMatchId: null,
  elapsedNowMs: undefined,
  markdownViewportPriority: true,
  assistantUserMessageId: null,
  retryUserMessage: null,
  assistantActionsAlwaysVisible: false,
  assistantCompletionTimeOnly: false,
  assistantRetryLabel: undefined,
  mcpAppInlineAnchorsEnabled: true,
  readOnly: false,
  turnRunId: null,
  fileChangesVisible: false,
  canSend: false,
  canBranchFromUserMessage: () => true,
  isRetryableUserMessage: () => true,
  readTurnTerminal: () => undefined,
  resolveSystemNoticeText: () => undefined,
  readSubagentTarget: () => null,
})

const emit = defineEmits<{
  toggleDetails: [turnId: string]
  toggleBlock: [blockId: string]
  toggleBlockGroup: [groupId: string]
  'edit-message': [payload: ChatUiTimelineMessageActionPayload<ChatUiTimelineRowMessage>]
  'retry-message': [payload: ChatUiTimelineMessageActionPayload<ChatUiTimelineRowMessage>]
  'fork-message': [message: ChatUiTimelineForkMessage]
  'branch-message': [message: { id: string; role: 'user' }]
  'preview-file': [target: ChatUiTimelinePreviewTarget]
  'commit': [paths: string[]]
  'commit-and-push': [paths: string[]]
  'push': []
  'diff-file': [path: string, messageId: string]
  'open-agent': [target: ChatUiTimelineSubagentTarget]
}>()

const hostTimeline = useOptionalChatUiHost()?.timeline
const timelineActions = hostTimeline?.actions
const turnOutputComponent: Component | null = hostTimeline?.slots?.turnOutput ?? null
const fileChangesVisible = computed(() => (
  hostTimeline?.effects?.visibility?.fileChanges?.value ?? props.fileChangesVisible
))
const actionPending = computed(() => timelineActions?.pending.value === true)
const fallbackTerminal = computed(() => props.row.kind === 'message'
  ? props.readTurnTerminal(props.row.message)
  : undefined)

const assistantIsRunning = computed(() => (
  props.row.kind === 'turn'
  && (props.row.turn.isRunning || (props.isLatestTurn === true && props.timelineRunning === true))
))

const resolvedRetryUserMessage = computed(() => {
  if (props.row.kind !== 'turn' || assistantIsRunning.value) {
    return null
  }
  const message = props.retryUserMessage ?? props.row.turn.userMessage
  if (!message || message.role !== 'user' || message.content.trim().length === 0) {
    return null
  }
  // Retry resubmits through the same cut before this row, so a queued or steer
  // input must not offer it once its turn has output.
  if (!props.isRetryableUserMessage(message)) return null
  return message
})

const canRetry = computed(() => !props.readOnly
  && resolvedRetryUserMessage.value !== null
  && (timelineActions
    ? timelineActions.canEdit(resolvedRetryUserMessage.value.id) === true
    : true))
const retryDisabled = computed(() => timelineActions
  ? !canRetry.value || actionPending.value
  : props.readOnly || !props.sessionId || !props.canSend)
const canForkAssistant = computed(() => (
  !props.readOnly
  && props.row.kind === 'turn'
  && props.row.turn.forkAnchorMessageId !== null
  && props.row.turn.forkAfterSequence != null
  && (timelineActions
    ? timelineActions.canFork(props.row.turn.forkAnchorMessageId) === true
    : true)
))
// Branch from here is a per-turn action: the cut is before the user message
// that anchors the turn, and a running turn has no completed rest point.
const canBranchAssistant = computed(() => (
  props.row.kind === 'turn'
  && props.row.turn.userMessage !== null
  && !props.readOnly
  && props.canBranchFromUserMessage(props.row.turn.userMessage)
))
// A completed historical turn can still carry a `sending` user message on an
// older snapshot; only the live session runtime proves the turn is mid-run.
const branchDisabled = computed(() => (
  actionPending.value
  || (assistantIsRunning.value && props.timelineRunning === true)
))

function isTimelineSystemNoticeMessage(message: ChatUiTimelineRowMessage): boolean {
  const hostNotice = hostTimeline?.hooks?.isSystemNoticeMessage
  if (hostNotice !== undefined) {
    return hostNotice(message)
  }
  return props.resolveSystemNoticeText(message) !== undefined
}

function systemNoticeText(message: ChatUiTimelineRowMessage): string {
  const text = hostTimeline?.hooks?.systemNoticeText?.(message)
    ?? props.resolveSystemNoticeText(message)
  if (text === undefined) {
    throw new Error(`Timeline system notice requires resolver text: ${message.id}`)
  }
  return text
}

const assistantNotificationTextByBlockId = computed<Record<string, string>>(() => {
  if (props.row.kind !== 'turn') {
    return {}
  }
  const textByBlockId: Record<string, string> = {}
  for (const block of props.row.turn.blocks) {
    if (block.kind !== 'notification') {
      continue
    }
    if (!block.notificationMessage) {
      throw new Error(`Timeline notification block requires source message: ${block.id}`)
    }
    textByBlockId[block.id] = systemNoticeText(block.notificationMessage)
  }
  return textByBlockId
})

const assistantSubagentTargetByBlockId = computed<Record<string, ChatUiTimelineSubagentTarget>>(() => {
  if (props.readOnly || props.row.kind !== 'turn') {
    return {}
  }
  const targetByBlockId: Record<string, ChatUiTimelineSubagentTarget> = {}
  for (const block of props.row.turn.blocks) {
    if (block.kind !== 'notification' || !block.notificationMessage) {
      continue
    }
    const target = props.readSubagentTarget(block)
    if (target !== null) {
      targetByBlockId[block.id] = target
    }
  }
  return targetByBlockId
})

function emitAssistantRetry(): void {
  if (retryDisabled.value) return
  const message = resolvedRetryUserMessage.value
  if (!message) {
    throw new Error('Completed assistant turn requires a retry user message')
  }
  emit('retry-message', {
    message,
    content: message.content,
  })
}

function sequenceFromOrderKey(orderKey: string | undefined): number | undefined {
  const sequence = Number(orderKey)
  return Number.isSafeInteger(sequence) && sequence >= 0 ? sequence : undefined
}

function emitUserFork(): void {
  if (props.readOnly) return
  if (props.row.kind !== 'turn' || props.row.turn.userMessage === null) {
    throw new Error('Completed assistant turn requires a user fork message')
  }
  const message = props.row.turn.userMessage
  emit('fork-message', {
    id: message.id,
    role: 'user',
    content: message.content,
    attachments: message.attachments,
    metadata: message.metadata,
    sequence: sequenceFromOrderKey(message.orderKey),
    message,
  })
}

function emitBranchMessage(): void {
  if (!canBranchAssistant.value || branchDisabled.value) return
  if (props.row.kind !== 'turn' || props.row.turn.userMessage === null) {
    throw new Error('Completed assistant turn requires a branch anchor')
  }
  emit('branch-message', { id: props.row.turn.userMessage.id, role: 'user' })
}

function emitAssistantFork(): void {
  if (!canForkAssistant.value || actionPending.value) return
  if (props.row.kind !== 'turn' || props.row.turn.forkAnchorMessageId === null) {
    throw new Error('Completed assistant turn requires a fork anchor')
  }
  emit('fork-message', {
    id: props.row.turn.forkAnchorMessageId,
    role: 'assistant',
    content: undefined,
    sequence: props.row.turn.forkAfterSequence ?? undefined,
  })
}
</script>

<template>
  <template v-if="row.kind === 'turn'">
    <div class="group">
      <component
        :is="userMessageComponent"
        v-if="row.turn.userMessage"
        :message="row.turn.userMessage"
        :search-matches-by-source="searchMatchesBySource"
        :active-search-match-id="activeSearchMatchId"
        :read-only="readOnly"
        :can-branch="canBranchAssistant"
        :branch-disabled="branchDisabled"
        @edit-message="emit('edit-message', $event)"
        @branch-message="emitBranchMessage"
        @fork-message="emitUserFork"
      />

      <div
        v-if="row.turn.blocks.length > 0 || row.turn.terminal !== undefined"
        :data-timeline-assistant-for-user-message-id="assistantUserMessageId ?? undefined"
      >
        <component
          :is="assistantMessageComponent"
          :session-id="sessionId"
          :blocks="row.turn.blocks"
          :model="row.turn.model"
          :details-expanded="turnDetailsExpanded"
          :details-toggle-visible="turnDetailsToggleVisible"
          :block-expanded-by-id="blockExpandedById"
          :block-group-expanded-by-id="blockGroupExpandedById"
          :notification-text-by-block-id="assistantNotificationTextByBlockId"
          :subagent-target-by-notification-block-id="assistantSubagentTargetByBlockId"
          :terminal="row.turn.terminal ?? null"
          :read-only="readOnly"
          :can-show-work-duration="row.turn.canShowWorkDuration"
          :work-duration-ms="row.turn.workDurationMs"
          :turn-started-at="row.turn.turnStartedAt"
          :elapsed-now-ms="elapsedNowMs"
          :search-query="searchQuery"
          :search-matches-by-source="searchMatchesBySource"
          :active-search-match-id="activeSearchMatchId"
          :markdown-viewport-priority="markdownViewportPriority ?? true"
          :is-running="assistantIsRunning"
          :mcp-app-inline-anchors-enabled="readOnly ? false : mcpAppInlineAnchorsEnabled"
          @toggle-details="emit('toggleDetails', row.turn.id)"
          @toggle-block="blockId => emit('toggleBlock', blockId)"
          @toggle-block-group="groupId => emit('toggleBlockGroup', groupId)"
          @open-agent="emit('open-agent', $event)"
          @preview-file="emit('preview-file', $event)"
        />
      </div>

      <div
        class="collapse-grid"
        :class="{ 'collapse-grid-open': !assistantIsRunning }"
        data-testid="timeline-footer-region"
      >
        <div class="collapse-grid-inner space-y-2" :inert="assistantIsRunning || undefined">
          <component
            :is="artifactsComponent"
            v-if="!readOnly && turnRunId !== null && props.isLatestTurn"
            :session-id="sessionId"
            :run-id="turnRunId"
            :anchor-message-id="row.turn.userMessage?.id ?? ''"
            :is-running="assistantIsRunning"
            @preview-file="emit('preview-file', $event)"
            @commit="emit('commit', $event)"
            @commit-and-push="emit('commit-and-push', $event)"
            @push="emit('push')"
          />

          <component
            :is="fileChangesComponent"
            v-if="!readOnly && fileChangesVisible && turnRunId !== null && row.turn.userMessage && props.isLatestTurn"
            :session-id="sessionId"
            :run-id="turnRunId"
            :anchor-message-id="row.turn.userMessage.id"
            :is-running="assistantIsRunning"
            @diff-file="(path, msgId) => emit('diff-file', path, msgId)"
          />

          <component v-if="turnOutputComponent" :is="turnOutputComponent" :message-ids="row.turn.messageIds" />

          <component
            :is="assistantActionsComponent"
            v-if="row.turn.blocks.length > 0 || row.turn.terminal !== undefined"
            :blocks="row.turn.blocks"
            :is-running="assistantIsRunning"
            :fork-message-id="canForkAssistant ? row.turn.forkAnchorMessageId : null"
            :fork-disabled="actionPending"
            :can-retry="canRetry"
            :retry-disabled="retryDisabled"
            :actions-always-visible="assistantActionsAlwaysVisible"
            :completion-time-only="assistantCompletionTimeOnly"
            :retry-label="assistantRetryLabel"
            :completed-at="row.turn.turnEndedAt"
            @fork-message="emitAssistantFork"
            @retry-message="emitAssistantRetry"
          />
        </div>
      </div>
    </div>
  </template>

  <TimelineWindowTruncationDivider
    v-else-if="row.kind === 'display' && row.row.kind === 'window_truncation'"
    :row="row.row"
  />

  <component
    v-else-if="row.kind === 'display'"
    :is="compressionDividerComponent"
    :row="row.row"
  />

  <component
    v-else-if="row.kind === 'message' && row.message.role === 'divider'"
    :is="compressionDividerComponent"
    :message="row.message"
  />

  <component
    :is="assistantMessageComponent"
    v-else-if="row.kind === 'message' && fallbackTerminal"
    :terminal="fallbackTerminal"
    :read-only="readOnly"
    :details-expanded="true"
    :details-toggle-visible="false"
    :block-expanded-by-id="blockExpandedById"
    :search-matches-by-source="searchMatchesBySource"
    :active-search-match-id="activeSearchMatchId"
    @toggle-block="blockId => emit('toggleBlock', blockId)"
  />

  <div
    v-else-if="row.kind === 'message' && isTimelineSystemNoticeMessage(row.message)"
    class="mb-2 space-y-1.5"
  >
    <span class="whitespace-pre-wrap text-base text-muted-foreground">
      {{ systemNoticeText(row.message) }}
    </span>
  </div>

  <div v-else-if="row.kind === 'message'" class="mb-2">
    <span class="whitespace-pre-wrap text-base text-muted-foreground">{{ row.message.content }}</span>
  </div>
  <component v-if="turnOutputComponent && row.kind === 'message'" :is="turnOutputComponent" :message-ids="[row.message.id]" />
</template>

<style scoped>
.collapse-grid {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 300ms cubic-bezier(0.16, 1, 0.3, 1);
}

.collapse-grid-open {
  grid-template-rows: 1fr;
}

.collapse-grid-inner {
  overflow: hidden;
  min-height: 0;
  opacity: 0;
  transition: opacity 300ms ease-in;
}

.collapse-grid-open .collapse-grid-inner {
  opacity: 1;
}
</style>
