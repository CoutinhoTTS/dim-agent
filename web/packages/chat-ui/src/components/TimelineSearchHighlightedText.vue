<script setup lang="ts">
import { computed } from 'vue'
import type { ChatUiTimelineSearchMatch } from '../timeline/search'

interface TextSegment {
  key: string
  text: string
  match?: ChatUiTimelineSearchMatch
}

const props = withDefaults(
  defineProps<{
    text: string
    matches: readonly ChatUiTimelineSearchMatch[]
    activeMatchId?: string | null
    baseOffset?: number
  }>(),
  {
    activeMatchId: null,
    baseOffset: 0,
  },
)

const segments = computed<TextSegment[]>(() => {
  if (!props.matches.length || !props.text) {
    return [{ key: 'text:0', text: props.text }]
  }

  const textStart = props.baseOffset
  const textEnd = props.baseOffset + props.text.length
  const scopedMatches = props.matches
    .filter(match => match.startOffset < textEnd && match.endOffset > textStart)
    .map(match => ({
      match,
      start: Math.max(0, match.startOffset - textStart),
      end: Math.min(props.text.length, match.endOffset - textStart),
    }))
    .filter(({ start, end }) => start < end)
    .sort((a, b) => a.start - b.start)

  if (!scopedMatches.length) {
    return [{ key: 'text:0', text: props.text }]
  }

  const nextSegments: TextSegment[] = []
  let cursor = 0

  scopedMatches.forEach(({ match, start, end }, index) => {
    if (cursor < start) {
      nextSegments.push({
        key: `text:${cursor}:${start}`,
        text: props.text.slice(cursor, start),
      })
    }

    nextSegments.push({
      key: `match:${match.id}:${index}`,
      text: props.text.slice(start, end),
      match,
    })
    cursor = end
  })

  if (cursor < props.text.length) {
    nextSegments.push({
      key: `text:${cursor}:end`,
      text: props.text.slice(cursor),
    })
  }

  return nextSegments
})
</script>

<template>
  <template v-for="segment in segments" :key="segment.key">
    <span
      v-if="segment.match"
      :data-timeline-search-match-id="segment.match.id"
      :class="[
        'timeline-search-match',
        segment.match.id === activeMatchId ? 'timeline-search-match-active' : '',
      ]"
    >{{ segment.text }}</span>
    <span v-else>{{ segment.text }}</span>
  </template>
</template>

<style>
.timeline-search-match {
  border-radius: 3px;
  background: color-mix(in srgb, #facc15 45%, transparent);
  color: inherit;
}

.timeline-search-match-active {
  background: color-mix(in srgb, #f59e0b 70%, transparent);
  box-shadow: 0 0 0 1px color-mix(in srgb, #d97706 70%, transparent);
}
</style>
