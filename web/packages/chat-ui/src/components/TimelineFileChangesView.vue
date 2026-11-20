<script setup lang="ts">
import { computed, ref, type Component } from 'vue'
import type {
  ChatUiFileChangeIcons,
  ChatUiFileChangeLabels,
  ChatUiFileChangeRow,
  ChatUiFileChangeStatusKind,
} from './file-changes'

defineOptions({
  name: 'TimelineFileChangesView',
})

const props = defineProps<{
  rows: readonly ChatUiFileChangeRow[]
  summaryCount: number
  isUndone: boolean
  showUndo: boolean
  showReapply: boolean
  canUndo: boolean
  actionPending: boolean
  previewLoading: boolean
  labels: ChatUiFileChangeLabels
  icons: ChatUiFileChangeIcons
  localActionsAvailable?: boolean
  diffDisabled?: boolean
  buttonComponent?: Component
  contextMenuComponent: Component
  contextMenuTriggerComponent: Component
  contextMenuContentComponent: Component
  contextMenuItemComponent: Component
  contextMenuSeparatorComponent: Component
}>()

const emit = defineEmits<{
  expand: [open: boolean]
  diff: [row: ChatUiFileChangeRow]
  undo: []
  reapply: []
  'open-file': [row: ChatUiFileChangeRow]
  'show-in-folder': [row: ChatUiFileChangeRow]
  'copy-absolute': [row: ChatUiFileChangeRow]
  'copy-relative': [row: ChatUiFileChangeRow]
}>()

const buttonComponent = computed<Component | string>(() => props.buttonComponent ?? 'button')
const expanded = ref(false)
const mutationDisabled = computed(() => !props.canUndo || props.actionPending)
const summaryText = computed(() => props.isUndone
  ? props.labels.filesChangedUndone
  : props.labels.fileChangesCount(props.summaryCount))

const KIND_LETTERS: Record<ChatUiFileChangeStatusKind, string> = {
  modified: 'M',
  added: 'A',
  deleted: 'D',
}

const KIND_LETTER_CLASSES: Record<ChatUiFileChangeStatusKind, string> = {
  modified: 'text-activity-warning',
  added: 'text-activity-success',
  deleted: 'text-destructive',
}

function toggleExpanded(): void {
  expanded.value = !expanded.value
  emit('expand', expanded.value)
}

function handleFileClick(row: ChatUiFileChangeRow): void {
  if (!props.diffDisabled && !props.isUndone && !row.reportedOnly && row.scope !== 'external') {
    emit('diff', row)
  }
}

function rowStatsText(row: ChatUiFileChangeRow): string {
  if (row.reportedOnly) {
    return ''
  }
  if (row.insertions === undefined && row.deletions === undefined) {
    return props.labels.binary
  }
  const parts: string[] = []
  if (row.insertions !== undefined) {
    parts.push(`+${row.insertions}`)
  }
  if (row.deletions !== undefined) {
    parts.push(`-${row.deletions}`)
  }
  return parts.join('/')
}
</script>

<template>
  <div v-if="rows.length > 0 && summaryCount > 0" data-testid="timeline-file-changes">
    <div class="flex items-center justify-between gap-2">
      <button
        type="button"
        class="flex min-w-0 items-center gap-1.5 text-sm transition-colors"
        :class="isUndone ? 'text-muted-foreground/60' : 'text-muted-foreground hover:text-foreground'"
        data-testid="timeline-file-changes-toggle"
        @click="toggleExpanded"
      >
        <component :is="expanded ? icons.chevronDown : icons.chevronRight" class="h-4 w-4 shrink-0" />
        <span class="truncate">{{ summaryText }}</span>
      </button>

      <component
        :is="buttonComponent"
        v-if="showUndo"
        variant="link"
        size="sm"
        class="h-auto shrink-0 p-0 text-sm font-medium text-muted-foreground hover:text-destructive"
        data-testid="timeline-file-changes-undo"
        :disabled="mutationDisabled"
        @click="emit('undo')"
      >
        <component :is="previewLoading ? icons.loader2 : icons.rotateCcw" class="mr-1 h-3.5 w-3.5" :class="{ 'animate-spin': previewLoading }" />
        {{ labels.undo }}
      </component>

      <component
        :is="buttonComponent"
        v-else-if="showReapply"
        variant="link"
        size="sm"
        class="h-auto shrink-0 p-0 text-sm font-medium text-muted-foreground hover:text-foreground"
        data-testid="timeline-file-changes-reapply"
        :disabled="mutationDisabled"
        @click="emit('reapply')"
      >
        <component :is="icons.redo2" class="mr-1 h-3.5 w-3.5" />
        {{ labels.reapply }}
        <component :is="icons.loader2" v-if="actionPending" class="ml-1 h-3 w-3 animate-spin" />
      </component>
    </div>

    <div
      v-if="expanded"
      class="mt-0.5 overflow-hidden rounded-md border border-border"
      :class="{ 'opacity-50': isUndone }"
      data-testid="timeline-file-changes-list"
    >
      <component :is="contextMenuComponent" v-for="row in rows" :key="row.path">
        <component :is="contextMenuTriggerComponent" as-child>
          <button
            type="button"
            class="flex w-full min-w-0 items-center gap-3 border-b border-border px-2 py-1.5 text-left text-sm transition-colors last:border-b-0"
            :class="isUndone || row.reportedOnly ? 'cursor-default' : 'hover:bg-accent/40'"
            data-testid="timeline-file-changes-file"
            :data-file-path="row.path"
            :disabled="diffDisabled"
            @click="handleFileClick(row)"
          >
            <span
              class="w-5 shrink-0 text-center font-mono text-xs font-semibold"
              :class="row.statusKind ? KIND_LETTER_CLASSES[row.statusKind] : 'text-muted-foreground'"
              data-testid="timeline-file-changes-status"
            >{{ row.statusKind ? KIND_LETTERS[row.statusKind] : '?' }}</span>
            <span class="min-w-0 flex-1 truncate text-muted-foreground" :title="row.path">
              {{ row.path }}
            </span>
            <span class="shrink-0 font-mono text-xs text-muted-foreground/80">{{ rowStatsText(row) }}</span>
          </button>
        </component>
        <component :is="contextMenuContentComponent" class="w-48">
          <component
            :is="contextMenuItemComponent"
            data-testid="timeline-file-changes-context-open"
            v-if="localActionsAvailable"
            @select="emit('open-file', row)"
          >
            <component :is="icons.externalLink" class="mr-2 h-4 w-4" />
            <span>{{ labels.openWithDefault }}</span>
          </component>
          <component
            :is="contextMenuItemComponent"
            data-testid="timeline-file-changes-context-show-in-folder"
            v-if="localActionsAvailable"
            @select="emit('show-in-folder', row)"
          >
            <component :is="icons.folderOpen" class="mr-2 h-4 w-4" />
            <span>{{ labels.revealInFolder }}</span>
          </component>
          <component :is="contextMenuSeparatorComponent" v-if="localActionsAvailable" />
          <component
            :is="contextMenuItemComponent"
            data-testid="timeline-file-changes-context-copy-absolute"
            v-if="row.absolutePath"
            @select="emit('copy-absolute', row)"
          >
            <component :is="icons.copy" class="mr-2 h-4 w-4" />
            <span>{{ labels.copyPath }}</span>
          </component>
          <component
            :is="contextMenuItemComponent"
            data-testid="timeline-file-changes-context-copy-relative"
            v-if="row.scope === 'workspace'"
            @select="emit('copy-relative', row)"
          >
            <component :is="icons.copy" class="mr-2 h-4 w-4" />
            <span>{{ labels.copyRelativePath }}</span>
          </component>
        </component>
      </component>
    </div>
  </div>
</template>
