<script setup lang="ts">
import { computed } from 'vue'
import type { ChatUiFileChipOpenPayload } from './file-chip'
import { toFileDisplayLabel } from '../utils/file-display-label'

defineOptions({
  name: 'FileChip',
})

const props = withDefaults(defineProps<{
  /** Absolute path (file:// chips, absolute inline-code paths). */
  absolutePath?: string
  /** Workspace-relative path (Markdown relative links, inline-code paths). */
  relativeHref?: string
  /** Directory (no file extension) → folder icon + system file manager. */
  isDirectory?: boolean
  /** Optional display label override — used to strip serialization artifacts like @ prefix. */
  label?: string
  /** Host-resolved icon URL; the package does not own file icon assets. */
  fileIconUrl?: string
  /** Host-driven pending state for the open action. */
  opening?: boolean
}>(), {
  absolutePath: undefined,
  relativeHref: undefined,
  isDirectory: false,
  label: undefined,
  fileIconUrl: undefined,
  opening: false,
})

const emit = defineEmits<{
  open: [payload: ChatUiFileChipOpenPayload]
}>()

/** The file path; drives the open action and the hover tooltip. */
const path = computed(() => props.absolutePath || props.relativeHref || '')

/** Only the file name is shown inline; the full path lives in the tooltip. */
const fileName = computed(() => props.label ?? toFileDisplayLabel(path.value, props.isDirectory))

function handleClick(event: MouseEvent) {
  if (!path.value || props.opening) return
  emit('open', {
    absolutePath: props.absolutePath,
    relativeHref: props.relativeHref,
    isDirectory: props.isDirectory,
    external: event.metaKey || event.ctrlKey,
  })
}
</script>

<template>
  <button
    type="button"
    class="file-chip"
    :class="{ 'file-chip--disabled': !path || opening }"
    :disabled="!path || opening"
    :title="path"
    :data-file-absolute="absolutePath || undefined"
    :data-file-relative="relativeHref || undefined"
    :data-file-directory="isDirectory ? 'true' : undefined"
    @click.prevent.stop="handleClick($event)"
  >
    <img class="file-chip__icon" :src="fileIconUrl" alt="" aria-hidden="true" />
    <span class="file-chip__label">{{ fileName }}</span>
  </button>
</template>

<style scoped>
.file-chip {
  display: inline-flex;
  align-items: center;
  gap: 0.2em;
  max-width: 100%;
  border: 0;
  background: none;
  padding: 0;
  font-size: inherit;
  color: var(--link-color);
  cursor: pointer;
  vertical-align: middle;
}

.file-chip:hover:not(:disabled) .file-chip__label {
  text-decoration: underline;
  text-underline-offset: 0.2em;
}

.file-chip--disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.file-chip__icon {
  width: 1em;
  height: 1em;
  flex-shrink: 0;
}

.file-chip__label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
