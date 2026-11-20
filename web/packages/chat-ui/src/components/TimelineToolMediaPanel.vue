<script setup lang="ts">
import { computed } from 'vue'
import { extractCuaVisualResults } from '../timeline/generated-image-results'
import type {
  ChatUiToolMediaImageCard,
  ChatUiToolMediaPanelBlock,
  ChatUiToolMediaPanelLabels,
} from './tool-media-panel'

/**
 * Expanded CUA tool media panel. The host supplies the labels and image card;
 * this component owns the screenshot/expandable action split, input projection,
 * and output formatting.
 */
defineOptions({
  name: 'TimelineToolMediaPanel',
})

const props = defineProps<{
  block: ChatUiToolMediaPanelBlock
  labels: ChatUiToolMediaPanelLabels
  imageCardComponent: ChatUiToolMediaImageCard
}>()

const CUA_SCREENSHOT_ACTIONS = new Set(['get_window_state', 'zoom', 'health_report'])
const CUA_EXPANDABLE_ACTIONS = new Set([
  'list_apps', 'list_windows', 'get_accessibility_tree',
  'get_screen_size', 'get_cursor_position', 'page',
])

const inputKeys = [
  'name', 'bundle_id', 'pid', 'window_id', 'element_token', 'element_index',
  'x', 'y', 'text', 'value', 'key', 'keys', 'direction', 'amount',
  'session', 'enabled', 'action', 'selector', 'css_selector', 'javascript',
  'url', 'from_x', 'from_y', 'to_x', 'to_y', 'x1', 'y1', 'x2', 'y2',
] as const

const images = computed(() => extractCuaVisualResults(props.block))

const panel = computed<{ inputLines: string[]; output: string; hasImages: boolean } | null>(() => {
  const toolName = (props.block.toolName ?? '').trim().toLowerCase()
  // Strip the cua-driver__ prefix.
  const action = toolName.startsWith('cua-driver__')
    ? toolName.slice('cua-driver__'.length)
    : toolName
  const hasImages = images.value.length > 0
  const isScreenshot = CUA_SCREENSHOT_ACTIONS.has(action)
  const isExpandable = CUA_EXPANDABLE_ACTIONS.has(action)

  if (!isScreenshot && !isExpandable && !hasImages) return null

  const params = asRecord(props.block.toolParams)
  const inputLines: string[] = []
  if (params) {
    for (const key of inputKeys) {
      const value = params[key]
      if (value === undefined || value === null) continue
      inputLines.push(`${key}: ${typeof value === 'string' ? value : JSON.stringify(value)}`)
    }
  }

  const output = isScreenshot ? '' : formatOutput(props.block.toolResult)
  return { inputLines, output: output || '', hasImages }
})

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

function formatRaw(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function formatOutput(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  const record = asRecord(value)
  if (record) {
    const content = record.content
    if (typeof content === 'string' && content.trim().length > 0) return content
  }
  return formatRaw(value)
}
</script>

<template>
  <div
    v-if="panel"
    class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
    data-testid="timeline-tool-cua-panel"
  >
    <template v-if="panel.hasImages">
      <component
        :is="imageCardComponent"
        :images="images.slice(0, 3)"
        compact
        :show-actions="false"
      />
      <div v-if="images.length > 3" class="px-3 pb-1 text-xs text-muted-foreground/70">
        +{{ images.length - 3 }}
      </div>
    </template>
    <template v-if="panel.inputLines.length > 0">
      <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">{{ labels.input }}</div>
      <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ panel.inputLines.join('\n') }}</pre>
    </template>
    <div v-if="panel.inputLines.length > 0 && panel.output" class="border-t border-border"></div>
    <template v-if="panel.output">
      <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">{{ labels.output }}</div>
      <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ panel.output }}</pre>
    </template>
  </div>
</template>
