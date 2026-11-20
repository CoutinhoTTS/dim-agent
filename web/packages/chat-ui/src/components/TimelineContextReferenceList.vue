<script setup lang="ts">
import type { Component } from 'vue'
import ContextReferenceHoverContent from './ContextReferenceHoverContent.vue'
import type {
  ChatUiContextReferenceIcons,
  ChatUiContextReferenceLabels,
  ChatUiSentReferenceMetadata,
} from './context-references'

defineOptions({
  name: 'TimelineContextReferenceList',
})

defineProps<{
  references: readonly ChatUiSentReferenceMetadata[]
  labels: ChatUiContextReferenceLabels
  icons: ChatUiContextReferenceIcons
  hoverCardComponent: Component
  hoverCardTriggerComponent: Component
  hoverCardContentComponent: Component
}>()

function sourceIcon(
  icons: ChatUiContextReferenceIcons,
  source: ChatUiSentReferenceMetadata['source'],
): Component {
  return icons.source[source]
}
</script>

<template>
  <div
    class="flex max-w-full flex-wrap justify-end gap-x-1.5 gap-y-1"
    data-testid="timeline-context-references"
  >
    <component
      :is="hoverCardComponent"
      v-for="(reference, index) in references"
      :key="index"
      :open-delay="200"
      :close-delay="80"
    >
      <component :is="hoverCardTriggerComponent" as-child>
        <span
          class="inline-flex h-7 max-w-full items-center gap-1.5 rounded-lg border border-border/70 bg-muted px-2 text-sm text-foreground/80"
          :data-testid="`timeline-context-reference-${index + 1}`"
        >
          <component :is="sourceIcon(icons, reference.source)" class="size-3.5 shrink-0" />
          <span class="truncate">{{ reference.source === 'conversation' ? 'annotation' : reference.text }}</span>
        </span>
      </component>
      <component
        :is="hoverCardContentComponent"
        v-if="reference.selectedText !== undefined || reference.comment !== undefined"
        side="top"
        align="end"
        class="w-96 max-w-[calc(100vw-2rem)] overflow-hidden p-0"
      >
        <ContextReferenceHoverContent
          :position="index + 1"
          :selected-text="reference.selectedText"
          :comment="reference.comment"
          :labels="labels"
          :icons="icons"
        />
      </component>
    </component>
  </div>
</template>
