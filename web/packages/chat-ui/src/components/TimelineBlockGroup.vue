<script setup lang="ts">
import { computed, ref, watch, TransitionGroup, type Component } from 'vue'
import { useTimelineBlockGroupTransition } from '../timeline/block-group-transition'
import { useThinkingExcerpt } from '../timeline/use-thinking-excerpt'
import type {
  ChatUiBackgroundTaskNotice,
  ChatUiTimelineBlockGroupBlock,
  ChatUiTimelineBlockGroupIcons,
  ChatUiTimelineBlockGroupLabels,
  ChatUiTimelineBlockGroupSummaryItem,
  ChatUiTimelineSubagentTarget,
  ChatUiTimelineToolSummaryCategory,
} from './block-group'

defineOptions({
  name: 'TimelineBlockGroup',
})

const props = withDefaults(defineProps<{
  groupId: string
  blocks: readonly ChatUiTimelineBlockGroupBlock[]
  expanded: boolean
  blockExpandedById: Readonly<Record<string, boolean>>
  notificationTextByBlockId: Readonly<Record<string, string>>
  subagentTargetByNotificationBlockId: Readonly<Record<string, ChatUiTimelineSubagentTarget>>
  toolCategoryByBlockId: Readonly<Record<string, ChatUiTimelineToolSummaryCategory>>
  elapsedNowMs?: number
  sessionId: string | null
  mcpAppInlineAnchorsEnabled: boolean
  transitionEnabled: boolean
  labels: ChatUiTimelineBlockGroupLabels
  icons: ChatUiTimelineBlockGroupIcons
  buttonComponent: Component
  tooltipComponent: Component
  tooltipTriggerComponent: Component
  tooltipContentComponent: Component
  thinkingItemComponent: Component
  toolCallItemComponent: Component
  mcpAppInlineAnchorComponent: Component
  providerIconComponent: Component
  readBackgroundTaskNotice: (block: ChatUiTimelineBlockGroupBlock) => ChatUiBackgroundTaskNotice | null
  getAgentModelLabel: (info: {
    agentName?: string
    providerId?: string
    modelId?: string
    externalAgentType?: string
  }) => string | null
  loadExecOutput?: (block: ChatUiTimelineBlockGroupBlock, outputPath: string) => Promise<string>
}>(), {
  elapsedNowMs: undefined,
  loadExecOutput: undefined,
})

const emit = defineEmits<{
  'toggle-block': [blockId: string]
  'toggle-block-group': [groupId: string]
  'open-agent': [target: ChatUiTimelineSubagentTarget]
  'preview-file': [target: { path: string; startLine?: number }]
}>()

const isRunning = computed(() =>
  props.blocks.some(block => block.status === 'running'),
)

// A collapsed large group is only a summary row. The shared transition keeps
// its tool rows mounted during the close animation, then unmounts them; small
// groups stay mounted to avoid remount churn.
const {
  contentMounted,
  contentOpen,
} = useTimelineBlockGroupTransition({
  expanded: () => props.expanded,
  blockCount: () => props.blocks.length,
})

const thinkingCount = computed(() =>
  props.blocks.filter(block => block.kind === 'thinking').length,
)

// A collapsed running group must still say what the model is thinking. Only
// the newest work block counts: notifications are status side notes, never the
// current phase. Once a tool starts, its row is the current activity and the
// previous thinking excerpt must not pose as one.
const activeThinkingBlock = computed(() => {
  if (!isRunning.value) return undefined
  for (let index = props.blocks.length - 1; index >= 0; index -= 1) {
    const block = props.blocks[index]!
    if (block.kind === 'notification') continue
    return block.kind === 'thinking' && block.status === 'running' ? block : undefined
  }
  return undefined
})
const {
  text: groupExcerpt,
  truncated: groupExcerptTruncated,
  flush: flushGroupExcerpt,
} = useThinkingExcerpt(
  () => activeThinkingBlock.value?.content ?? '',
  () => activeThinkingBlock.value !== undefined,
)
// Consecutive thinking parts keep `active` true; flush so the previous part's
// tail cannot stay on screen until the next throttle window.
watch(() => activeThinkingBlock.value?.id, () => flushGroupExcerpt())
const showGroupExcerpt = computed(() => !props.expanded && groupExcerpt.value !== '')

const toolBlocks = computed(() =>
  props.blocks.filter(block => block.kind === 'tool'),
)

const notificationCount = computed(() =>
  props.blocks.filter(block => block.kind === 'notification').length,
)

const CATEGORY_ORDER: ChatUiTimelineToolSummaryCategory[] = [
  'read', 'edit', 'command', 'search', 'webSearch', 'webFetch', 'subagent', 'browser', 'computerUse', 'wait', 'skill', 'unknown',
]

const summaryItems = computed<ChatUiTimelineBlockGroupSummaryItem[]>(() => {
  const items: ChatUiTimelineBlockGroupSummaryItem[] = []

  if (thinkingCount.value > 0) {
    items.push({
      key: 'thinking',
      count: thinkingCount.value,
      separator: '',
    })
  }

  const categoryCounts: Partial<Record<ChatUiTimelineToolSummaryCategory, number>> = {}
  for (const block of toolBlocks.value) {
    const category = props.toolCategoryByBlockId[block.id] ?? 'unknown'
    categoryCounts[category] = (categoryCounts[category] ?? 0) + 1
  }

  for (const category of CATEGORY_ORDER) {
    const count = categoryCounts[category]
    if (count === undefined) continue
    const previous = items.at(-1)
    items.push({
      key: category,
      count,
      separator: previous === undefined
        ? ''
        : props.labels[previous.key === 'thinking' ? 'separator' : 'workActionSeparator'],
    })
  }

  if (notificationCount.value > 0) {
    items.push({
      key: 'notifications',
      count: notificationCount.value,
      separator: items.length > 0 ? props.labels.separator : '',
    })
  }

  return items
})

function handleToggleClick() {
  emit('toggle-block-group', props.groupId)
}

// ── Enter stagger ───────────────────────────────────────────
// New children (and the initial appear burst) enter with a per-index delay so
// a freshly upgraded group rolls in instead of popping in fully formed.
// Delay caps at STAGGER_MAX_INDEX so long groups never wait long.
const STAGGER_MAX_INDEX = 8
const STAGGER_STEP_MS = 40

function setStaggerDelay(element: Element) {
  const index = Number((element as HTMLElement).dataset.enterIndex ?? 0)
  ;(element as HTMLElement).style.transitionDelay = `${index * STAGGER_STEP_MS}ms`
}

// The rail is a `TransitionGroup` while the enter animation is on, and a plain
// element when it is off. Off drops the enter/leave/move hooks together with
// the position reads those hooks perform (`TransitionGroup` measures every
// mounted child on each of its renders), which is the single factor the
// performance harness prices against the animated rail.
// A run can upgrade from a single work row to a group while it streams; that
// mounts a new rail whose children were already visible as single rows. Do not
// replay the enter animation for that initial set (`appear: false`), otherwise
// the row fades/slides in again and reads as a redraw. Children added after the
// rail is mounted still use the stagger enter.
const groupRailComponent = computed<Component | string>(() =>
  props.transitionEnabled ? TransitionGroup : 'div',
)
const groupRailTransitionProps = computed(() => props.transitionEnabled
  ? {
      tag: 'div',
      name: 'group-item',
      appear: false,
      onBeforeEnter: setStaggerDelay,
    }
  : {})

// ── Subagent notification detection ─────────────────────────

function noticeParams(block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null {
  return props.readBackgroundTaskNotice(block)
}

function readSubagentNotice(block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null {
  const params = noticeParams(block)
  if (!params || params.toolName !== 'agent' || params.agentName === undefined) return null
  return params
}

function readExecTaskNotice(block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null {
  const params = noticeParams(block)
  if (!params) return null
  // Subagent notifications are handled separately
  if (params.toolName === 'agent' && params.agentName !== undefined) return null
  return params
}

function isSubagentNotification(block: ChatUiTimelineBlockGroupBlock): boolean {
  return readSubagentNotice(block) !== null
}

function isExecTaskNotification(block: ChatUiTimelineBlockGroupBlock): boolean {
  return readExecTaskNotice(block) !== null
}

function execTaskNoticeParams(block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null {
  return readExecTaskNotice(block)
}

function execTaskStatusText(block: ChatUiTimelineBlockGroupBlock): string {
  const params = readExecTaskNotice(block)
  if (!params) return ''
  return props.labels.backgroundTaskStatus[params.status]
}

const execOutputCache = ref<Record<string, { content: string; loading: boolean; error?: string }>>({})

async function loadExecOutput(block: ChatUiTimelineBlockGroupBlock) {
  const params = readExecTaskNotice(block)
  if (!props.loadExecOutput || !params?.outputPath || !props.sessionId) return
  if (execOutputCache.value[block.id]) return

  execOutputCache.value[block.id] = { content: '', loading: true }
  try {
    const content = await props.loadExecOutput(block, params.outputPath)
    execOutputCache.value[block.id] = { content, loading: false }
  } catch (e) {
    execOutputCache.value[block.id] = { content: '', loading: false, error: e instanceof Error ? e.message : String(e) }
  }
}

function toggleExecOutput(block: ChatUiTimelineBlockGroupBlock) {
  const cached = execOutputCache.value[block.id]
  if (!cached) {
    void loadExecOutput(block)
  }
  emit('toggle-block', block.id)
}

function subagentNoticeParams(block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null {
  return readSubagentNotice(block)
}

function agentModelLabel(block: ChatUiTimelineBlockGroupBlock): string | null {
  return props.getAgentModelLabel(subagentNoticeParams(block) ?? {})
}

function hasSubagentSummary(block: ChatUiTimelineBlockGroupBlock): boolean {
  return subagentSummary(block).length > 0
}

function subagentSummary(block: ChatUiTimelineBlockGroupBlock): string {
  // Extract <result> content from the notification message XML.
  // The notice.params.summary only carries the status string (e.g. "cancelled"),
  // while the full subagent output lives in block.content as:
  //   <notification>...<result>子agent输出</result>...</notification>
  const content = block.content
  const match = content.match(/<result[^>]*>([\s\S]*?)<\/result>/)
  if (match?.[1]) {
    return match[1].trim()
  }
  // Fallback to notice.params.summary (status string)
  const params = readSubagentNotice(block)
  return params?.summary?.trim() ?? ''
}

function isMcpAppBlock(block: ChatUiTimelineBlockGroupBlock): boolean {
  return block.kind === 'tool' && block.toolSource?.kind === 'mcp'
}

function requiredNotificationText(block: ChatUiTimelineBlockGroupBlock): string {
  const text = props.notificationTextByBlockId[block.id]
  if (text === undefined) {
    throw new Error(`Timeline notification block requires formatted text: ${block.id}`)
  }
  return text
}
</script>

<template>
  <div class="mb-3" data-testid="timeline-block-group" data-group-kind="work" :data-expanded="expanded ? 'true' : 'false'">
    <component
      :is="buttonComponent"
      variant="ghost"
      class="group/block-group !p-0 h-auto w-full min-w-0 justify-start gap-1 rounded-none text-base font-normal text-muted-foreground shadow-none transition-colors hover:bg-transparent dark:hover:bg-transparent hover:text-foreground"
      data-testid="timeline-block-group-toggle"
      @click="handleToggleClick"
    >
      <component
        :is="icons.workflow"
        class="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-colors group-hover/block-group:text-foreground mr-0.5"
        data-testid="timeline-block-group-icon"
      />
      <span
        class="min-w-0 truncate"
        :class="{ 'shimmer-text': isRunning }"
        data-testid="timeline-block-group-summary"
      >
        <template v-for="item in summaryItems" :key="item.key">{{ item.separator }}<span :data-summary-category="item.key"><slot name="summary-item" :item="item">{{ item.count }} {{ item.key }}</slot></span></template>
      </span>
      <component
        :is="icons.chevron"
        class="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover/block-group:text-foreground"
        :class="{ 'rotate-90': expanded }"
      />
    </component>

    <div
      v-if="showGroupExcerpt"
      class="timeline-group-excerpt"
      :class="{ 'has-source-gate': groupExcerptTruncated }"
      :title="groupExcerpt"
      aria-hidden="true"
      data-testid="timeline-block-group-excerpt"
    >
      <span class="timeline-group-excerpt-text" data-testid="timeline-block-group-excerpt-text">
        <bdi dir="ltr">{{ groupExcerpt }}</bdi>
      </span>
    </div>

    <div
      v-if="contentMounted"
      class="collapse-grid"
      :class="{ 'collapse-grid-open': contentOpen }"
      data-testid="timeline-block-group-content"
    >
      <div class="collapse-grid-inner">
        <component
          :is="groupRailComponent"
          v-bind="groupRailTransitionProps"
          class="border-l border-border/60 pt-1 pl-3 ml-1.5 overflow-hidden"
          data-testid="timeline-block-group-rail"
        >
          <template v-for="(block, blockIndex) in blocks" :key="block.id">
            <component
              :is="thinkingItemComponent"
              v-if="block.kind === 'thinking'"
              :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
              :block="block"
              :expanded="blockExpandedById[block.id] ?? false"
              :elapsed-started-at="block.startedAt"
              :elapsed-now-ms="elapsedNowMs"
              :settled-duration-ms="block.durationMs"
              @toggle="emit('toggle-block', block.id)"
            />
            <template v-else-if="block.kind === 'tool'">
              <component
                :is="toolCallItemComponent"
                :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
                :block="block"
                :expanded="blockExpandedById[block.id] ?? false"
                :session-id="sessionId"
                @toggle="emit('toggle-block', block.id)"
                @preview-file="emit('preview-file', $event)"
              />
              <component
                :is="mcpAppInlineAnchorComponent"
                v-if="mcpAppInlineAnchorsEnabled && sessionId && isMcpAppBlock(block)"
                :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
                :session-id="sessionId"
                :block="block"
              />
            </template>
            <!-- Subagent notification: agent chip + status + expandable summary -->
            <div
              v-else-if="block.kind === 'notification' && isSubagentNotification(block)"
              :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
              class="mb-3"
              data-testid="timeline-notification-block"
              :data-block-id="block.id"
            >
              <component
                :is="buttonComponent"
                variant="ghost"
                class="group/tool-call !p-0 h-auto min-w-0 flex-1 justify-start gap-1 rounded-none text-base font-normal shadow-none transition-colors hover:bg-transparent dark:hover:bg-transparent"
                :class="{ 'cursor-default': !hasSubagentSummary(block) }"
                @click="hasSubagentSummary(block) && emit('toggle-block', block.id)"
              >
                <component :is="icons.bot" class="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-colors group-hover/tool-call:text-foreground mr-0.5" />
                <component :is="tooltipComponent">
                  <component :is="tooltipTriggerComponent" as-child>
                    <span class="inline-flex shrink-0 items-center gap-1 rounded-md bg-foreground/[0.06] px-1.5 py-0.5">
                      <component
                        :is="providerIconComponent"
                        :provider-id="subagentNoticeParams(block)?.providerId"
                        :model-id="subagentNoticeParams(block)?.modelId"
                        :external-agent-type="subagentNoticeParams(block)?.externalAgentType"
                        :size="14"
                      />
                      <span class="text-sm font-medium text-foreground">{{ subagentNoticeParams(block)?.agentName }}</span>
                    </span>
                  </component>
                  <component :is="tooltipContentComponent" v-if="agentModelLabel(block)">
                    {{ agentModelLabel(block) }}
                  </component>
                </component>
                <span class="text-base text-muted-foreground">
                  {{ requiredNotificationText(block) }}
                </span>
                <component
                  :is="icons.chevron"
                  v-if="hasSubagentSummary(block)"
                  class="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover/tool-call:text-foreground"
                  :class="{ 'rotate-90': blockExpandedById[block.id] ?? false }"
                />
              </component>
              <div
                v-if="hasSubagentSummary(block)"
                class="collapse-grid"
                :class="{ 'collapse-grid-open': blockExpandedById[block.id] ?? false }"
              >
                <div class="collapse-grid-inner">
                  <div class="py-1">
                    <div class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card">
                      <pre class="px-3 py-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ subagentSummary(block) }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </div>
            <!-- Exec background task notification: terminal icon + label + status + expandable output -->
            <div
              v-else-if="block.kind === 'notification' && isExecTaskNotification(block)"
              :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
              class="mb-3"
              data-testid="timeline-notification-block"
              :data-block-id="block.id"
            >
              <component
                :is="buttonComponent"
                variant="ghost"
                class="group/tool-call !p-0 h-auto min-w-0 flex-1 justify-start gap-1 rounded-none text-base font-normal shadow-none transition-colors hover:bg-transparent dark:hover:bg-transparent"
                @click="toggleExecOutput(block)"
              >
                <component :is="icons.terminal" class="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-colors group-hover/tool-call:text-foreground mr-0.5" />
                <span class="shrink-0 text-muted-foreground transition-colors group-hover/tool-call:text-foreground">{{ execTaskNoticeParams(block)?.label ?? labels.backgroundTaskCommand }}</span>
                <span class="shrink-0 text-muted-foreground/70">{{ execTaskStatusText(block) }}</span>
                <component
                  :is="icons.chevron"
                  v-if="execTaskNoticeParams(block)?.outputPath"
                  class="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover/tool-call:text-foreground"
                  :class="{ 'rotate-90': blockExpandedById[block.id] ?? false }"
                />
              </component>
              <div
                v-if="execTaskNoticeParams(block)?.outputPath"
                class="collapse-grid"
                :class="{ 'collapse-grid-open': blockExpandedById[block.id] ?? false }"
              >
                <div class="collapse-grid-inner">
                  <div class="py-1">
                    <div class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card">
                      <div v-if="execOutputCache[block.id]?.loading" class="px-3 py-2 text-xs text-muted-foreground">Loading...</div>
                      <div v-else-if="execOutputCache[block.id]?.error" class="px-3 py-2 text-xs text-activity-warning">{{ execOutputCache[block.id].error }}</div>
                      <pre v-else class="px-3 py-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ execOutputCache[block.id]?.content }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </div>
            <div
              v-else-if="block.kind === 'notification'"
              :data-enter-index="Math.min(blockIndex, STAGGER_MAX_INDEX)"
              class="mb-3 flex items-center gap-1 text-base text-muted-foreground"
              data-testid="timeline-notification-block"
              :data-block-id="block.id"
            >
              <component :is="icons.layers" class="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <button
                v-if="subagentTargetByNotificationBlockId[block.id]"
                type="button"
                class="inline-flex min-w-0 items-center gap-1 text-left hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                :aria-label="`${labels.open}: ${requiredNotificationText(block)}`"
                data-testid="timeline-subagent-chat-open"
                @click="emit('open-agent', subagentTargetByNotificationBlockId[block.id]!)"
              >
                <span>{{ requiredNotificationText(block) }}</span>
                <component :is="icons.chevron" class="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              </button>
              <template v-else>
                {{ requiredNotificationText(block) }}
              </template>
            </div>
          </template>
        </component>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
  animation: shimmer 2s steps(48, end) infinite;
}

@media (prefers-reduced-motion: reduce) {
  .shimmer-text {
    animation: none;
    background: none;
    -webkit-text-fill-color: currentColor;
  }
}

@keyframes shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}

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

/* Mirrors the collapsed thinking row for LTR-script reasoning: short text stays
   complete and left-aligned, overflow is clipped from the left with an ellipsis. */
.timeline-group-excerpt {
  display: flex;
  align-items: center;
  gap: 3px;
  min-width: 0;
  margin: 2px 0 2px 20px;
  overflow: hidden;
  font-size: 0.875rem;
  line-height: 1.35;
  color: var(--muted-foreground);
}

/* The source was longer than the bounded fragment, so mark the missing
   earlier text even when the fragment itself fits the row. Native
   `text-overflow` cannot know about text that was never rendered. */
.timeline-group-excerpt.has-source-gate::before {
  content: "…";
  flex: none;
  color: var(--muted-foreground);
  font-family: var(--font-mono);
  font-size: 10px;
  pointer-events: none;
}

.timeline-group-excerpt-text {
  display: block;
  min-width: 0;
  flex: 1;
  overflow: hidden;
  white-space: nowrap;
  text-align: left;
  /* RTL direction puts the clip and native ellipsis on the left, keeping the
     newest characters visible. The inner <bdi dir="ltr"> pins the fragment to
     an LTR paragraph, so a fragment that starts with an RTL run still exposes
     its newest LTR tail and trailing punctuation. */
  direction: rtl;
  text-overflow: ellipsis;
}

.timeline-group-excerpt.has-source-gate .timeline-group-excerpt-text {
  /* The generated leading column is the marker; avoid a second ellipsis. */
  text-overflow: clip;
}

.group-item-enter-active {
  transition: opacity 240ms ease-out, transform 240ms cubic-bezier(0.16, 1, 0.3, 1);
}

.group-item-enter-from {
  opacity: 0;
  transform: translateY(6px);
}

.group-item-enter-to {
  opacity: 1;
  transform: translateY(0);
}

.group-item-leave-active {
  transition: opacity 150ms ease-in;
}

.group-item-leave-to {
  opacity: 0;
}
</style>
