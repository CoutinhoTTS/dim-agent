import { computed, ref, toValue, type MaybeRefOrGetter } from 'vue'

export type ChatUiCompressionStatus =
  | 'running'
  | 'completed'
  | 'failed'
  | 'skipped'
  | 'cancelled'

export interface ChatUiCompressionInfo {
  id: string
  status: ChatUiCompressionStatus
  /** The underlying payload completed, even if the message itself failed. */
  completed: boolean
  partial: boolean
  auto: boolean
  sessionId?: string
  segmentId?: string
  errorMessage?: string
  failureCode?: string
}

export interface ChatUiCompressionLabels {
  compressing: string
  failed: string
  skipped: string
  cancelled: string
  partial: string
  autoCompacted: string
  compressed: string
  summaryLoading: string
  summaryUnavailable: string
  summaryLoadFailed: string
  retry: string
}

export type ChatUiCompressionSummaryResult =
  | { status: 'available'; summary: string }
  | { status: 'unavailable' }

export interface UseTimelineCompressionSummaryOptions {
  info: MaybeRefOrGetter<ChatUiCompressionInfo | null>
  labels: MaybeRefOrGetter<ChatUiCompressionLabels>
  loadSummary?: (input: { sessionId: string; segmentId: string }) => Promise<ChatUiCompressionSummaryResult>
}

export function useTimelineCompressionSummary(options: UseTimelineCompressionSummaryOptions) {
  const info = computed(() => toValue(options.info))
  const labels = computed(() => toValue(options.labels))

  const expanded = ref(false)
  const summaryStatus = ref<'idle' | 'loading' | 'available' | 'unavailable' | 'failed'>('idle')
  const summary = ref('')

  const failureErrorMessage = computed(() => {
    const current = info.value
    if (current?.status !== 'failed') return ''
    const errorMessage = current.errorMessage
    return errorMessage?.trim() ? errorMessage : ''
  })
  const hasFailureDetails = computed(() => failureErrorMessage.value.length > 0)
  const canExpand = computed(() =>
    hasFailureDetails.value
    || (
      info.value?.completed === true
      && Boolean(info.value.sessionId?.trim())
      && Boolean(info.value.segmentId?.trim())
    ),
  )
  const summaryPanelId = computed(() => `compression-summary-${info.value?.id || 'legacy'}`)

  const isRunning = computed(() => info.value?.status === 'running')
  const isFailed = computed(() => info.value?.status === 'failed')
  const isSkipped = computed(() => info.value?.status === 'skipped')
  const isCancelled = computed(() => info.value?.status === 'cancelled')
  const isPartial = computed(() => info.value?.partial === true)
  const isAuto = computed(() => info.value?.auto === true)

  const compressionText = computed(() => {
    if (isRunning.value) return labels.value.compressing
    if (isFailed.value) return labels.value.failed
    if (isSkipped.value) return labels.value.skipped
    if (isCancelled.value) return labels.value.cancelled
    const partialSuffix = isPartial.value ? ` (${labels.value.partial})` : ''
    const base = isAuto.value ? labels.value.autoCompacted : labels.value.compressed
    return `${base}${partialSuffix}`
  })

  async function loadSummary(): Promise<void> {
    const current = info.value
    const sessionId = current?.sessionId?.trim()
    const segmentId = current?.segmentId?.trim()
    if (!sessionId || !segmentId || summaryStatus.value === 'loading' || !options.loadSummary) return

    summaryStatus.value = 'loading'
    try {
      const result = await options.loadSummary({ sessionId, segmentId })
      if (result.status === 'unavailable') {
        summaryStatus.value = 'unavailable'
        return
      }
      summary.value = result.summary
      summaryStatus.value = 'available'
    }
    catch {
      summaryStatus.value = 'failed'
    }
  }

  function toggleSummary(): void {
    if (!canExpand.value) return
    expanded.value = !expanded.value
    if (expanded.value && !hasFailureDetails.value && summaryStatus.value === 'idle') {
      void loadSummary()
    }
  }

  return {
    expanded,
    summaryStatus,
    summary,
    failureErrorMessage,
    hasFailureDetails,
    canExpand,
    summaryPanelId,
    isRunning,
    isFailed,
    isSkipped,
    isCancelled,
    isPartial,
    isAuto,
    compressionText,
    loadSummary,
    toggleSummary,
  }
}
