<script setup lang="ts">
import { computed, type Component } from 'vue'
import {
  resolveToolCallInlineIconKey,
  type ChatUiToolCallInlineAgentInfo,
  type ChatUiToolCallInlineIcons,
  type ChatUiToolCallInlineLabels,
  type ChatUiToolCallInlineStatus,
  type ChatUiToolCallInlineTrace,
} from './tool-call-inline-summary'

defineOptions({
  name: 'ToolCallInlineSummary',
})

const props = defineProps<{
  trace: ChatUiToolCallInlineTrace
  status: ChatUiToolCallInlineStatus
  labels: ChatUiToolCallInlineLabels
  icons: ChatUiToolCallInlineIcons
  fileIconUrl?: string | null
  faviconHostname?: string | null
  agentInfo?: ChatUiToolCallInlineAgentInfo | null
  externalAgentType?: string | null
  agentModelLabel?: string | null
  isPopoutWindow?: boolean
  tooltipComponent: Component
  tooltipTriggerComponent: Component
  tooltipContentComponent: Component
  providerIconComponent: Component
  faviconComponent: Component
}>()

const emit = defineEmits<{
  'agent-chip-click': []
}>()

const titleText = computed(() => {
  const title = props.trace.parameterTitle ?? props.trace.parameterText
  return title ? `${props.trace.labelText} ${title}` : props.trace.labelText
})

// Glob listing (pattern=* or missing) shows a files icon;
// glob search and grep both show file-search.
const isGlobListing = computed(() =>
  props.trace.rendererId === 'search'
  && props.trace.rawToolName === 'glob'
  && props.trace.labelText === props.labels.listFiles,
)

const actionIcon = computed(() => {
  const key = resolveToolCallInlineIconKey(props.trace, isGlobListing.value)
  return key ? props.icons[key] ?? null : null
})
</script>

<template>
  <span
    class="inline-flex min-w-0 max-w-full items-center gap-1"
    :class="{ 'shimmer-text': status === 'running' }"
    :title="titleText"
    data-testid="timeline-tool-summary"
  >
    <component
      :is="actionIcon.component"
      v-if="actionIcon"
      v-bind="actionIcon.props"
      class="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-colors group-hover/tool-call:text-foreground mr-0.5"
      data-testid="timeline-tool-icon"
    />
    <span
      v-if="trace.rendererId !== 'wait'"
      class="shrink-0 text-muted-foreground transition-colors group-hover/tool-call:text-foreground"
      data-testid="timeline-tool-summary-label"
    >{{ trace.labelText }}</span>
    <img
      v-if="fileIconUrl"
      :src="fileIconUrl"
      class="h-3.5 w-3.5 shrink-0 object-contain"
      data-testid="timeline-tool-file-icon"
      alt=""
    >
    <component
      :is="faviconComponent"
      v-if="faviconHostname"
      :hostname="faviconHostname"
      :size="16"
    />
    <span v-if="agentInfo && isPopoutWindow" class="text-sm font-medium text-foreground">{{ agentInfo.agentName }}</span>
    <component :is="tooltipComponent" v-if="agentInfo && !isPopoutWindow">
      <component :is="tooltipTriggerComponent" as-child>
        <span
          class="inline-flex shrink-0 items-center gap-1 rounded-md bg-foreground/[0.06] px-1.5 py-0.5 cursor-pointer hover:bg-foreground/[0.1]"
          data-testid="timeline-tool-agent-chip"
          @click.stop="emit('agent-chip-click')"
        >
          <component
            :is="providerIconComponent"
            :provider-id="agentInfo.providerId"
            :model-id="agentInfo.modelId"
            :external-agent-type="agentInfo.externalAgentType"
            :size="14"
          />
          <span class="text-sm font-medium text-foreground">{{ agentInfo.agentName }}</span>
        </span>
      </component>
      <component :is="tooltipContentComponent" v-if="agentModelLabel">
        {{ agentModelLabel }}
      </component>
    </component>
    <span
      v-if="externalAgentType && !agentInfo"
      class="inline-flex shrink-0 items-center gap-1"
    >
      <component
        :is="providerIconComponent"
        :external-agent-type="externalAgentType"
        :size="14"
      />
      <span class="text-sm font-medium text-foreground">{{ externalAgentType }}</span>
    </span>
    <Transition name="param-swap" mode="out-in" appear>
      <span
        v-if="trace.parameterText"
        :key="trace.parameterText"
        class="param-enter-anim min-w-0 truncate text-muted-foreground/70 group-hover/tool-call:text-foreground/70"
        data-testid="timeline-tool-primary-parameter"
      >{{ trace.parameterText }}</span>
    </Transition>
    <span
      v-if="status === 'queued'"
      class="shrink-0 text-muted-foreground/70 group-hover/tool-call:text-foreground/70"
      data-testid="timeline-tool-queued"
    >{{ labels.queued }}</span>
  </span>
</template>
