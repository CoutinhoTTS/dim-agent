<script setup lang="ts">
import { computed, nextTick, onMounted, ref, useId, watch } from "vue"
import { useRoute, useRouter } from "vue-router"
import { useI18n } from "vue-i18n"
import { ArrowUp, Square } from "lucide-vue-next"
import type { ModelId } from "../../rpc/protocol"
import { useModelsStore } from "../../stores/models"
import { useSessionsStore } from "../../stores/sessions"
import ComposerModelMenu from "./ComposerModelMenu.vue"
import { quoteInto } from "./quote"

/**
 * The message composer: a growing text field, the model menu, and one button
 * that sends, or stops the run while one is in flight. Enter sends,
 * Shift+Enter breaks the line.
 */
const props = defineProps<{
  running: boolean
  stopping?: boolean
  model: ModelId | undefined
  /** False when this connection may not change the model; the menu then only names it. */
  modelSelectable: boolean
  /** Sends the text; resolves once the server accepted it. */
  send: (text: string) => Promise<void>
  stop?: () => Promise<void>
  selectModel: (model: ModelId) => Promise<void> | void
  /** The narrower line panel: no model menu, less vertical chrome. */
  variant?: "main" | "panel"
  placeholder?: string
  createBranch?: (prompt: string) => Promise<void>
  openLines?: () => void
}>()
const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const models = useModelsStore()
const sessions = useSessionsStore()
const field = ref<HTMLTextAreaElement>()
const modelMenu = ref<InstanceType<typeof ComposerModelMenu>>()
const menuId = useId()
const text = ref("")
const sending = ref(false)
const changingModel = ref(false)
const error = ref("")
const composing = ref(false)
const focused = ref(false)
const dismissed = ref(false)
const activeIndex = ref(0)

const availableCommands = computed(() => ([
  { category: "model", name: "models", available: props.modelSelectable && models.available && !changingModel.value },
  { category: "session", name: "branch", available: props.createBranch !== undefined },
  { category: "session", name: "lines", available: props.openLines !== undefined },
  { category: "session", name: "new", available: true },
] as const).filter((command) => command.available))
const commandPrefix = computed(() => /^\/[a-z]*$/i.test(text.value) ? text.value.slice(1).toLowerCase() : undefined)
const matches = computed(() => {
  const prefix = commandPrefix.value
  return prefix === undefined ? [] : availableCommands.value.filter((command) => command.name.startsWith(prefix))
})
const commandMode = computed(() => !composing.value && !dismissed.value && matches.value.length > 0)
const menuOpen = computed(() => focused.value && commandMode.value)

watch(text, () => {
  if (!text.value.startsWith("/")) dismissed.value = false
}, { flush: "sync" })
watch(matches, () => { activeIndex.value = 0 }, { flush: "sync" })

const showStop = computed(() => props.running && props.stop !== undefined && text.value.trim() === "")
const canSend = computed(() => text.value.trim() !== "" && !sending.value)

function resize(): void {
  const element = field.value
  if (element === undefined) return
  element.style.height = "auto"
  element.style.height = `${Math.min(element.scrollHeight, 300)}px`
}

async function submit(): Promise<void> {
  if (!canSend.value || composing.value) return
  if (commandMode.value) {
    const command = matches.value[activeIndex.value]
    if (command !== undefined) await runCommand(command.name)
    return
  }
  const message = text.value
  const branchCommand = !dismissed.value && /^\/branch(?:\s+([\s\S]*))?$/i.exec(message)
  if (branchCommand && !branchCommand[1]?.trim()) {
    await runCommand("branch")
    return
  }
  sending.value = true
  error.value = ""
  try {
    if (branchCommand) {
      if (!props.createBranch) throw new Error(t("composer.branchUnavailable"))
      await props.createBranch(branchCommand[1]!.trim())
    } else {
      await props.send(message)
    }
    if (text.value === message) text.value = ""
    await nextTick()
    resize()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    sending.value = false
  }
}

async function stop(): Promise<void> {
  error.value = ""
  try {
    await props.stop?.()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

async function onModel(model: ModelId): Promise<void> {
  changingModel.value = true
  error.value = ""
  try {
    await props.selectModel(model)
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    changingModel.value = false
  }
}

async function runCommand(name: typeof availableCommands.value[number]["name"]): Promise<void> {
  if (composing.value) return
  error.value = ""
  text.value = name === "branch" ? "/branch " : ""
  await nextTick()
  resize()
  if (name === "branch") field.value?.focus()
  else if (name === "models") modelMenu.value?.open()
  else if (name === "lines") props.openLines?.()
  else {
    try {
      const cwd = typeof route.params.sessionId === "string" ? sessions.byId.get(route.params.sessionId)?.cwd : undefined
      await router.push({ name: "chat-new", ...(cwd === undefined ? {} : { query: { cwd } }) })
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing || composing.value || event.keyCode === 229) return
  if (event.key === "Escape" && text.value.startsWith("/") && !dismissed.value) {
    event.preventDefault()
    event.stopPropagation()
    dismissed.value = true
    return
  }
  if (menuOpen.value && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
    event.preventDefault()
    activeIndex.value = (activeIndex.value + (event.key === "ArrowDown" ? 1 : -1) + matches.value.length) % matches.value.length
    return
  }
  if (event.key === "Enter" && !event.shiftKey && !event.altKey) {
    event.preventDefault()
    void submit()
  }
}

/** Quotes `message` at the end of the draft (see quote.ts). */
async function quote(message: string): Promise<void> {
  text.value = quoteInto(text.value, message)
  await nextTick()
  resize()
  const element = field.value
  element?.focus()
  element?.setSelectionRange(text.value.length, text.value.length)
}

onMounted(() => field.value?.focus())
defineExpose({ focus: () => field.value?.focus(), quote })
</script>

<template>
  <section class="composer-panel" :data-variant="variant ?? 'main'" :aria-label="t('chat.composer')">
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>
    <div class="composer-surface" @click.self="field?.focus()">
      <div v-if="menuOpen" :id="menuId" class="command-menu ui-popover" role="listbox" :aria-label="t('composer.commands')">
        <button
          v-for="(command, index) in matches"
          :id="`${menuId}-${index}`"
          :key="command.name"
          type="button"
          role="option"
          class="ui-menu-item command-item"
          :aria-selected="index === activeIndex"
          :data-highlighted="index === activeIndex ? '' : undefined"
          @mousedown.prevent
          @mouseenter="activeIndex = index"
          @click.stop="runCommand(command.name)"
        >
          <span class="command-category">{{ index === 0 || matches[index - 1]?.category !== command.category ? t(`composer.category.${command.category}`) : '' }}</span>
          <span class="command-name">/{{ command.name }}<span v-if="command.name === 'branch'" class="command-argument"> &lt;prompt&gt;</span></span>
          <span class="command-description">{{ t(`composer.command.${command.name}`) }}</span>
        </button>
        <p class="command-keys">{{ t("composer.commandKeys") }}</p>
      </div>
      <textarea
        ref="field"
        v-model="text"
        class="composer-input"
        rows="2"
        :placeholder="placeholder ?? t('composer.sendMessage')"
        :aria-label="placeholder ?? t('composer.sendMessage')"
        :aria-controls="menuOpen ? menuId : undefined"
        :aria-expanded="menuOpen"
        :aria-activedescendant="menuOpen ? `${menuId}-${activeIndex}` : undefined"
        @input="resize"
        @keydown="onKeydown"
        @focus="focused = true"
        @blur="focused = false"
        @compositionstart="composing = true"
        @compositionend="composing = false"
      />
      <footer class="composer-toolbar">
        <div class="composer-tool-group" />
        <div class="composer-run-group">
          <ComposerModelMenu v-if="variant !== 'panel'" ref="modelMenu" :model="model" :selectable="modelSelectable" :disabled="changingModel" @select="onModel" />
          <button
            class="ui-icon-button"
            :data-variant="showStop ? 'contrast' : 'primary'"
            data-size="sm"
            type="button"
            :aria-label="showStop ? t('composer.stopResponse') : t('composer.send')"
            v-tooltip="showStop ? t('composer.stopResponse') : t('composer.send')"
            :disabled="showStop ? stopping : !canSend"
            @click.stop="showStop ? stop() : submit()"
          >
            <Square v-if="showStop" fill="currentColor" aria-hidden="true" />
            <ArrowUp v-else :stroke-width="2.25" aria-hidden="true" />
          </button>
        </div>
      </footer>
    </div>
    <div v-if="$slots.status" class="composer-statusbar">
      <div class="composer-status-start"><slot name="status" /></div>
    </div>
  </section>
</template>

<style scoped>
.composer-input { display: block; width: 100%; min-height: 60px; max-height: 300px; resize: none; padding: 0; border: 0; background: transparent; color: var(--foreground); font: var(--text-base)/var(--leading-relaxed) var(--font-sans); outline: none; }
.composer-input::placeholder { color: var(--muted-foreground); }
.composer-panel[data-variant="panel"] { padding: var(--space-2) var(--space-3) var(--space-3); }
.composer-panel[data-variant="panel"] .composer-input { min-height: 48px; }
.command-menu { position: absolute; bottom: calc(100% + var(--space-2)); left: 0; width: 100%; max-width: none; max-height: min(320px, 50dvh); overflow-y: auto; transform-origin: bottom left; --popover-enter-y: 4px; }
.command-item { display: grid; grid-template-columns: 48px 128px minmax(0, 1fr); }
.command-category, .command-description { color: var(--muted-foreground); }
.command-category { font-size: var(--text-xs); }
.command-name { font-family: var(--font-mono); }
.command-argument { color: var(--muted-foreground); font-size: var(--text-xs); }
.command-keys { margin: var(--space-1) 0 0; padding: var(--space-2); border-top: 1px solid var(--border); color: var(--muted-foreground); font-size: var(--text-xs); }

</style>
