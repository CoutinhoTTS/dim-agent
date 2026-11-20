<script setup lang="ts">
import type { Component } from 'vue'
import type { EditReplacement } from '../timeline/tool-renderers/edit-replacements'

const props = withDefaults(defineProps<{
  replacements: EditReplacement[]
  fileName: string
  /** Host code/diff renderer; desktop supplies its themed `ToolCodeBlock`. */
  codeBlockComponent: Component
  labels: {
    replacement: (index: number, count: number) => string
    replaceAll: string
  }
  showIndexes?: boolean
  showLineNumbers?: boolean
}>(), {
  showIndexes: true,
  showLineNumbers: false,
})

const showHeader = (replacement: EditReplacement): boolean =>
  props.showIndexes || replacement.replaceAll

const replacementLabel = (replacement: EditReplacement): string =>
  props.labels.replacement(replacement.index + 1, props.replacements.length)
</script>

<template>
  <div data-testid="edit-replacements-list">
    <div
      v-for="replacement in replacements"
      :key="replacement.index"
      :data-testid="`edit-replacement-${replacement.index}`"
      class="border-t border-border first:border-t-0"
    >
      <div
        v-if="showHeader(replacement)"
        class="flex items-center gap-2 px-3 pt-2 pb-1 text-[11px] text-muted-foreground"
      >
        <span v-if="showIndexes">{{ replacementLabel(replacement) }}</span>
        <span
          v-if="replacement.replaceAll"
          data-testid="edit-replace-all-badge"
          class="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-600 dark:text-amber-400"
        >{{ labels.replaceAll }}</span>
      </div>
      <component
        :is="codeBlockComponent"
        :file-name="fileName"
        :old-code="replacement.oldText"
        :new-code="replacement.newText"
        :show-line-numbers="showLineNumbers"
      />
    </div>
  </div>
</template>
