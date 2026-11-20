<script setup lang="ts">
import type { ChatUiTimelineUserMark } from './user-message-marks'

defineOptions({
  name: 'TimelineUserMarks',
})

const props = defineProps<{
  marks: readonly ChatUiTimelineUserMark[]
}>()

const emit = defineEmits<{
  'mark-click': [mark: ChatUiTimelineUserMark]
}>()

function handleMarkClick(mark: ChatUiTimelineUserMark): void {
  emit('mark-click', mark)
}
</script>

<template>
  <div class="mb-1 space-y-1">
    <div
      v-for="mark in props.marks"
      :key="mark.num"
      class="w-full min-w-0 overflow-hidden rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-sm text-foreground"
      :class="mark.interactive ? 'cursor-pointer transition-colors hover:border-primary/40 hover:bg-primary/10' : ''"
      :data-testid="`timeline-user-mark-${mark.num}`"
      @click.stop="handleMarkClick(mark)"
    >
      <div class="flex min-w-0 items-baseline gap-1.5">
        <span class="shrink-0 font-semibold text-primary text-xs">mark {{ mark.num }}</span>
        <span class="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">{{ mark.selector }}</span>
      </div>
      <div v-if="mark.textLabel" class="mt-0.5 truncate text-xs text-muted-foreground">
        {{ mark.textLabel }}
      </div>
      <div v-for="(note, i) in mark.notes" :key="i" class="mt-0.5 whitespace-pre-wrap break-words text-xs text-foreground/80">
        {{ note }}
      </div>
    </div>
  </div>
</template>
