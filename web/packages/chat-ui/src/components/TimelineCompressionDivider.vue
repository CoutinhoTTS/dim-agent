<script setup lang="ts">
import { computed, type Component } from 'vue'
import {
  useTimelineCompressionSummary,
  type ChatUiCompressionInfo,
  type ChatUiCompressionLabels,
  type ChatUiCompressionSummaryResult,
} from '../timeline/compression'

defineOptions({
  name: 'TimelineCompressionDivider',
})

const props = defineProps<{
  info: ChatUiCompressionInfo | null
  labels: ChatUiCompressionLabels
  loadSummary?: (input: { sessionId: string; segmentId: string }) => Promise<ChatUiCompressionSummaryResult>
  buttonComponent?: Component
}>()

const buttonComponent = computed<Component | string>(() => props.buttonComponent ?? 'button')

const {
  expanded,
  summaryStatus,
  summary,
  failureErrorMessage,
  hasFailureDetails,
  canExpand,
  summaryPanelId,
  isRunning,
  isFailed,
  compressionText,
  loadSummary,
  toggleSummary,
} = useTimelineCompressionSummary({
  info: () => props.info,
  labels: () => props.labels,
  loadSummary: input => props.loadSummary?.(input) ?? Promise.resolve({ status: 'unavailable' as const }),
})
</script>

<template>
  <div class="py-1" data-testid="compression-divider" :data-status="info?.status">
    <div class="flex min-h-7 items-center gap-3">
      <div class="h-px flex-1 bg-border" />
      <component
        :is="buttonComponent"
        v-if="canExpand"
        type="button"
        variant="ghost"
        class="h-7 min-w-0 max-w-[72%] shrink-0 gap-1.5 px-2 text-base font-normal text-muted-foreground shadow-none hover:text-foreground"
        :aria-expanded="expanded"
        :aria-controls="summaryPanelId"
        data-testid="compression-summary-toggle"
        @click="toggleSummary"
      >
        <slot name="status-icon" :failed="isFailed" :show-archive="!isFailed" />
        <span class="min-w-0 truncate">{{ compressionText }}</span>
        <slot name="chevron-icon" :expanded="expanded" />
      </component>
      <span
        v-else
        class="flex max-w-[72%] shrink-0 items-center gap-2 truncate text-base text-muted-foreground"
      >
        <slot name="status-icon" :failed="isFailed" :show-archive="!isFailed && !isRunning" />
        <span
          class="truncate"
          :class="{ 'shimmer-text': isRunning }"
        >
          {{ compressionText }}
        </span>
      </span>
      <div class="h-px flex-1 bg-border" />
    </div>

    <div
      v-if="canExpand && expanded"
      :id="summaryPanelId"
      class="compression-summary-panel mx-auto mt-2 w-full max-w-3xl rounded-md border bg-muted/30 px-4 py-3"
      data-testid="compression-summary-panel"
    >
      <div
        v-if="hasFailureDetails"
        class="space-y-2 whitespace-pre-wrap break-words text-sm"
        data-testid="compression-failure-details"
      >
        <div
          v-if="info?.failureCode"
          class="font-mono text-xs text-muted-foreground"
          data-testid="compression-failure-code"
        >
          {{ info.failureCode }}
        </div>
        <div data-testid="compression-failure-message">
          {{ failureErrorMessage }}
        </div>
      </div>
      <p
        v-else-if="summaryStatus === 'loading'"
        class="text-sm text-muted-foreground"
        aria-live="polite"
      >
        {{ labels.summaryLoading }}
      </p>
      <div
        v-else-if="summaryStatus === 'available'"
        class="whitespace-pre-wrap break-words text-sm"
        data-testid="compression-summary-plaintext"
      >
        {{ summary }}
      </div>
      <p
        v-else-if="summaryStatus === 'unavailable'"
        class="text-sm text-muted-foreground"
        aria-live="polite"
      >
        {{ labels.summaryUnavailable }}
      </p>
      <div
        v-else-if="summaryStatus === 'failed'"
        class="flex items-center justify-between gap-3"
        aria-live="polite"
      >
        <p class="text-sm text-muted-foreground">
          {{ labels.summaryLoadFailed }}
        </p>
        <component
          :is="buttonComponent"
          type="button"
          variant="outline"
          size="sm"
          @click="loadSummary"
        >
          {{ labels.retry }}
        </component>
      </div>
    </div>
  </div>
</template>

<style scoped>
.compression-summary-panel {
  max-height: min(45vh, 480px);
  overflow-y: auto;
}

.shimmer-text {
  background: linear-gradient(
    90deg,
    currentColor 0%,
    currentColor 40%,
    var(--activity-shimmer-highlight) 50%,
    currentColor 60%,
    currentColor 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: compression-shimmer 2s steps(48, end) infinite;
}

@media (prefers-reduced-motion: reduce) {
  .shimmer-text {
    animation: none;
    background: none;
    -webkit-text-fill-color: currentColor;
  }
}

@keyframes compression-shimmer {
  0% {
    background-position: 100% 0;
  }
  100% {
    background-position: -100% 0;
  }
}
</style>
