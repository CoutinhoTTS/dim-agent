import { computed, toValue, type MaybeRefOrGetter } from 'vue'
import type { ChatUiTimelineBlock } from '../model/timeline'
import {
  buildTimelineAssistantCopyPayload,
  hasTimelineAssistantCopyPayload,
  type ChatUiTimelineCopyLabels,
} from './assistant-actions'
import { formatTimelineTimestamp } from '../utils/timeline-time-format'

export interface UseTimelineAssistantFooterOptions {
  blocks: MaybeRefOrGetter<readonly ChatUiTimelineBlock[]>
  isRunning: MaybeRefOrGetter<boolean>
  canRetry: MaybeRefOrGetter<boolean>
  forkMessageId: MaybeRefOrGetter<string | null>
  isPopoutWindow: MaybeRefOrGetter<boolean>
  actionsAlwaysVisible: MaybeRefOrGetter<boolean>
  completedAt: MaybeRefOrGetter<Date | null>
  labels: MaybeRefOrGetter<ChatUiTimelineCopyLabels>
  locale: MaybeRefOrGetter<string>
  completionTimeOnly?: MaybeRefOrGetter<boolean>
}

/**
 * Footer action gating and copy/retry/fork payloads for an assistant message.
 *
 * The footer is only final when neither the message nor any block is running;
 * until then no action is offered even if retry/fork are otherwise allowed.
 */
export function useTimelineAssistantFooter(options: UseTimelineAssistantFooterOptions) {
  const blocks = computed(() => toValue(options.blocks))
  const labels = computed(() => toValue(options.labels))

  const hasRunningBlock = computed(() => blocks.value.some(block => block.status === 'running'))
  const isMessageFinal = computed(() => !toValue(options.isRunning) && !hasRunningBlock.value)
  const hasCopyPayload = computed(() =>
    isMessageFinal.value
    && hasTimelineAssistantCopyPayload(blocks.value, labels.value),
  )
  const copyPayload = computed(() =>
    buildTimelineAssistantCopyPayload(blocks.value, labels.value),
  )
  const hasActions = computed(() =>
    isMessageFinal.value
    && (
      toValue(options.canRetry)
      || hasCopyPayload.value
      || (toValue(options.forkMessageId) !== null && !toValue(options.isPopoutWindow))
      || (toValue(options.actionsAlwaysVisible) && toValue(options.completedAt) !== null)
    ),
  )
  const completedTimeText = computed(() => {
    const completedAt = toValue(options.completedAt)
    if (completedAt === null) return ''
    return formatTimelineTimestamp(completedAt, toValue(options.locale), new Date(), {
      timeOnly: toValue(options.completionTimeOnly) === true,
    })
  })

  return {
    hasRunningBlock,
    isMessageFinal,
    hasCopyPayload,
    copyPayload,
    hasActions,
    completedTimeText,
  }
}
