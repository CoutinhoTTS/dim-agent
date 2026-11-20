<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import type { ChatUiContextReferenceActionIcons, ChatUiContextReferenceLabels } from './context-references'

const props = withDefaults(defineProps<{
  position: number
  selectedText?: string
  comment?: string
  editable?: boolean
  labels: ChatUiContextReferenceLabels
  icons: ChatUiContextReferenceActionIcons
}>(), {
  editable: false,
})

const emit = defineEmits<{
  updateComment: [comment: string]
  remove: []
}>()

const editing = ref(false)
const draftComment = ref('')
const textareaRef = ref<HTMLTextAreaElement | null>(null)

watch(() => props.comment, (comment) => {
  if (!editing.value) draftComment.value = comment ?? ''
}, { immediate: true })

async function startEditing(): Promise<void> {
  draftComment.value = props.comment ?? ''
  editing.value = true
  await nextTick()
  textareaRef.value?.focus()
}

function cancelEditing(): void {
  draftComment.value = props.comment ?? ''
  editing.value = false
}

function saveComment(): void {
  if (!editing.value) return
  editing.value = false
  emit('updateComment', draftComment.value.trim())
}

function removeAnnotation(): void {
  editing.value = false
  emit('remove')
}

function handleEditKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    cancelEditing()
    return
  }
  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.isComposing) {
    event.preventDefault()
    saveComment()
  }
}
</script>

<template>
  <div
    class="relative max-h-80 overflow-auto px-4 py-3 text-sm leading-relaxed"
    data-testid="context-reference-hover-content"
  >
    <div v-if="editable" class="absolute right-2 top-2 flex items-center gap-0.5">
      <button
        v-if="editing"
        type="button"
        class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :aria-label="labels.saveAnnotation"
        @mousedown.prevent
        @click.stop="saveComment"
      >
        <component :is="icons.check" class="size-4" />
      </button>
      <button
        v-else
        type="button"
        class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :aria-label="labels.editAnnotation"
        @click.stop="startEditing"
      >
        <component :is="icons.pencil" class="size-4" />
      </button>
      <button
        type="button"
        class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
        :aria-label="labels.removeAnnotation"
        @mousedown.prevent
        @click.stop="removeAnnotation"
      >
        <component :is="icons.remove" class="size-4" />
      </button>
    </div>

    <div class="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-x-2">
      <div v-if="selectedText !== undefined" class="text-right text-muted-foreground">
        {{ position }}.
      </div>
      <div v-if="selectedText !== undefined" :class="editable ? 'pr-14' : ''">
        <div class="text-muted-foreground">
          {{ labels.selectedText }}:
        </div>
        <div class="mt-1 whitespace-pre-wrap break-words text-foreground" data-testid="context-reference-selected-text">
          {{ selectedText }}
        </div>
      </div>

      <div
        v-if="editable || comment !== undefined"
        class="col-start-2"
        :class="selectedText !== undefined ? 'mt-4' : (editable ? 'pr-14' : '')"
      >
        <div class="text-muted-foreground">
          {{ labels.userComment }}:
        </div>
        <textarea
          v-if="editing"
          ref="textareaRef"
          v-model="draftComment"
          class="field-sizing-content mt-1 max-h-[98px] min-h-[38px] w-full resize-none rounded-md border border-input bg-background px-2.5 py-2 text-sm leading-5 text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :aria-label="labels.userComment"
          @blur="saveComment"
          @keydown="handleEditKeydown"
        />
        <div
          v-else
          class="mt-1 min-h-5 whitespace-pre-wrap break-words text-foreground"
          data-testid="context-reference-comment"
        >
          {{ comment }}
        </div>
      </div>
    </div>
  </div>
</template>
