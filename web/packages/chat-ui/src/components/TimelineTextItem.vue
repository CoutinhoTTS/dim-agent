<script setup lang="ts">
import { computed, type Component } from 'vue'
import MarkdownRender from 'markstream-vue'
import TimelineSearchDomText from './TimelineSearchDomText.vue'
import type { ChatUiTimelineBlock } from '../model'
import type { ChatUiTimelineSearchMatch } from '../timeline/search'
import { normalizeMarkdownWindowsPaths } from '../utils/markdown-windows-path'

/**
 * Single assistant text block renderer. Rendering props and the markdown
 * performance profiles stay identical to the desktop owner so search
 * highlighting, streaming, and scroll handoff behave the same.
 */
const props = defineProps<{
  block: ChatUiTimelineBlock
  sessionId: string | null
  searchQuery: string
  searchMatches: readonly ChatUiTimelineSearchMatch[]
  activeSearchMatchId: string | null
  markdownViewportPriority: boolean
  markdownComponent?: Component
  isDark: boolean
  codeBlockLightTheme: string
  codeBlockDarkTheme: string
  codeBlockProps: Readonly<Record<string, unknown>>
  codeBlockOptions: Readonly<Record<string, unknown>>
}>()

const priorityMarkdownPerformanceProps = {
  deferNodesUntilVisible: true,
  initialRenderBatchSize: 16,
  renderBatchSize: 16,
  renderBatchDelay: 16,
  renderBatchBudgetMs: 4,
  renderBatchIdleTimeoutMs: 120,
  maxLiveNodes: 480,
  liveNodeBuffer: 96,
} as const
const backgroundMarkdownPerformanceProps = {
  deferNodesUntilVisible: true,
  initialRenderBatchSize: 4,
  renderBatchSize: 4,
  renderBatchDelay: 48,
  renderBatchBudgetMs: 2,
  renderBatchIdleTimeoutMs: 300,
  maxLiveNodes: 160,
  liveNodeBuffer: 24,
} as const
const streamingMarkdownPerformanceProps = {
  smoothStreaming: true,
  deferNodesUntilVisible: false,
  initialRenderBatchSize: 512,
  renderBatchSize: 512,
  renderBatchDelay: 0,
} as const
const markdownPerformanceProps = computed(() =>
  props.markdownViewportPriority
    ? priorityMarkdownPerformanceProps
    : backgroundMarkdownPerformanceProps,
)

// Stable markdown renderer identity: markstream prefixes every node key with
// `indexKey`, so it is bound from the first streaming frame and never changes
// when the block flips streaming -> completed. A changed `indexKey` remounts
// every node and redraws the whole text.
const markdownIndexKey = computed(() =>
  `${props.sessionId ?? 'session'}:${props.block.id}`,
)

// Markdown escapes strip the separator before `_` in Windows paths
// (`G:\Tangula\_lookbook\...` → `G:\Tangula_lookbook\...`). Normalize the
// separators before markstream sees the content so file links stay valid.
const markdownContent = computed(() => normalizeMarkdownWindowsPaths(props.block.content))

const markdownIndexKeyBinding = computed(() =>
  ({ indexKey: markdownIndexKey.value }),
)

type MarkdownRenderBindingsCacheEntry = {
  viewportPriority: boolean
  bindings: Record<string, unknown>
}
const markdownRenderBindingsCache = new WeakMap<ChatUiTimelineBlock, MarkdownRenderBindingsCacheEntry>()

function markdownRenderBindings(): Record<string, unknown> {
  const cached = markdownRenderBindingsCache.get(props.block)
  if (cached && cached.viewportPriority === props.markdownViewportPriority) {
    return cached.bindings
  }

  // One profile for every block: completed answers that contain a code fence
  // used to opt out of the batched/virtualized profile (`deferNodesUntilVisible:
  // false`, 512-node batches, inner virtualization off), which put a whole
  // heavy document — thousands of nodes — into the frame that mounts the row.
  // On a session switch that single frame is what the user waits on, so only
  // the streaming profile stays separate.
  const bindings = {
    ...markdownPerformanceProps.value,
    ...(props.block.status === 'running' ? streamingMarkdownPerformanceProps : {}),
    ...markdownIndexKeyBinding.value,
    viewportPriority: props.markdownViewportPriority,
  }
  markdownRenderBindingsCache.set(props.block, {
    viewportPriority: props.markdownViewportPriority,
    bindings,
  })
  return bindings
}
</script>

<template>
  <div
    class="mb-3 text-sm leading-relaxed text-foreground"
    data-testid="timeline-text-block"
    :data-block-id="block.id"
    :data-node-index="block.id"
    :data-scroll-follow-content="block.status === 'running' ? 'true' : undefined"
    :data-message-status="block.status === 'running' ? 'streaming' : undefined"
  >
    <TimelineSearchDomText
      :query="searchQuery"
      :matches="searchMatches"
      :active-match-id="activeSearchMatchId"
    >
      <component
        :is="markdownComponent ?? MarkdownRender"
        :key="block.id"
        v-bind="markdownRenderBindings()"
        :class="[
          'timeline-markdown',
          'timeline-markdown--assistant',
          { 'timeline-markdown--streaming': block.status === 'running' },
        ]"
        :content="markdownContent"
        :streaming="block.status === 'running'"
        :final="block.status !== 'running'"
        :fade="block.status === 'running'"
        :is-dark="isDark"
        :code-block-light-theme="codeBlockLightTheme"
        :code-block-dark-theme="codeBlockDarkTheme"
        :code-block-props="codeBlockProps"
        :code-block-options="codeBlockOptions"
      />
    </TimelineSearchDomText>
  </div>
</template>
