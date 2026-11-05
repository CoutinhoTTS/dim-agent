<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue"
import { useI18n } from "vue-i18n"
import type { Dispose, PanelActivity, PanelEnvironment, PanelModule } from "../../../../npm/ui"
import { readSessionMethods } from "../../rpc/read"
import { useAppearanceStore } from "../../stores/appearance"
import { useChatStore } from "../../stores/chat"
import { useConnectionStore } from "../../stores/connection"
import { useExtensionsStore } from "../../stores/extensions"
import { useUiStore } from "../../stores/ui"
import { capabilities, createPanelContext, panelTitle, resourceUrl } from "./context"
import { messages } from "./messages"

const props = defineProps<{ sessionId: string }>()
const { t, locale } = useI18n({ messages })
const connection = useConnectionStore()
const extensions = useExtensionsStore()
const chat = useChatStore()
const ui = useUiStore()
const appearance = useAppearanceStore()
const container = ref<HTMLElement>()
const surface = ref<HTMLElement>()
const failure = ref("")
const loading = ref(false)
const attempt = ref(0)
const branchId = computed(() => ui.activeLineTab[props.sessionId] ?? "main")
const view = computed(() => chat.views[props.sessionId])
const extension = computed(() => extensions.directory?.extensions.find(extension => extension.pluginName === ui.extension?.pluginName))
const panel = computed(() => extension.value?.panels.find(panel => panel.id === ui.extension?.panelId))
const title = computed(() => panel.value ? panelTitle(panel.value, locale.value) : t("extensions"))
const ended = computed(() => view.value?.status === "closed" || view.value?.lines.some(line => line.id === branchId.value && (line.state.kind.startsWith("ended") || line.state.kind === "creation-failed")))
const ready = computed(() => view.value?.status === "ready" && (branchId.value === "main" || view.value.branches[branchId.value]?.status === "ready"))
const status = computed(() => connection.phase !== "connected" ? t("disconnected") : ended.value ? t("ended") : !ready.value ? t("syncing") : !panel.value ? t("unavailable") : failure.value || (loading.value ? t("loading") : ""))
const activity = computed<PanelActivity>(() => {
  const current = view.value!
  const branch = branchId.value === "main" ? current : current.branches[branchId.value]!
  return Object.freeze({ branch: Object.freeze({ running: branch.running, stopping: branch.stopping }), sessionRunning: current.sessionRunning })
})
const environment = computed<PanelEnvironment>(() => Object.freeze({ sessionId: props.sessionId, branchId: branchId.value, theme: appearance.resolvedTheme, locale: locale.value }))
let dispose: Dispose | undefined

watch([() => props.sessionId, branchId], ([sessionId, branchId]) => { if (branchId !== "main") chat.ensureLine(sessionId, branchId) }, { immediate: true })
watch(surface, async element => {
  if (element instanceof HTMLDialogElement && !element.open) element.showModal()
  await nextTick()
  element?.focus()
}, { flush: "post" })
watch([container, () => extension.value?.generation, () => panel.value?.id, () => props.sessionId, branchId, ready, ended, () => connection.phase, attempt], async (_next, _old, onCleanup) => {
  let valid = true
  let cleanup: Dispose | undefined
  onCleanup(() => { valid = false; cleanup?.(); dispose = undefined })
  failure.value = ""
  loading.value = false
  const element = container.value
  const declaration = panel.value
  const current = extension.value
  if (!element || !declaration || !current || !ready.value || ended.value || connection.phase !== "connected") return
  const sessionId = props.sessionId
  const branch = branchId.value
  const snapshot = environment.value
  const root = element.shadowRoot ?? element.attachShadow({ mode: "open" })
  root.replaceChildren()
  let binding: ReturnType<typeof createPanelContext> | undefined
  let unmount: Dispose | undefined
  cleanup = () => {
    binding?.dispose()
    try { unmount?.() } catch (error) { console.error("Extension cleanup failed", error) }
    unmount = undefined
    root.replaceChildren()
  }
  dispose = () => { valid = false; cleanup?.() }
  loading.value = true
  try {
    if (declaration.placement !== "session-panel") throw new Error(`Unsupported placement: ${declaration.placement}`)
    for (const capability of declaration.requires) {
      if (!capabilities.includes(capability)) throw new Error(`Unsupported capability: ${capability}`)
      const method = { "session.history": "session.history", "session.submit": "session.submit", "session.interrupt": "session.stop" }[capability]
      if (method && !connection.supports(method)) throw new Error(`Unavailable capability: ${capability}`)
    }
    const methods = declaration.methods.length === 0 ? [] : await connection.client.call("session.methods", { sessionId }, readSessionMethods)
    if (!valid) return
    for (const method of declaration.methods) if (!methods.includes(method.name)) throw new Error(`Unavailable method: ${method.name}`)
    const module: PanelModule = await import(/* @vite-ignore */ resourceUrl(current.generation, declaration.entry))
    if (!valid) return
    if (typeof module.mount !== "function") throw new Error("The extension does not export mount")
    binding = createPanelContext(declaration, {
      client: connection.client,
      environment: snapshot,
      observeEnvironment: listener => watch(environment, value => listener(value), { immediate: true, flush: "sync" }),
      observeActivity: listener => watch(activity, value => listener(value), { immediate: true, flush: "sync" }),
      unavailable(error) { failure.value = error.message; cleanup?.() },
    })
    unmount = module.mount(root, binding.context)
    if (typeof unmount !== "function") { unmount = undefined; throw new Error("mount must return a cleanup function") }
    if (sessionId !== props.sessionId || branch !== branchId.value) cleanup()
  } catch (error) {
    if (valid) { failure.value = error instanceof Error ? error.message : String(error); cleanup() }
  } finally { if (valid) loading.value = false }
}, { immediate: true, flush: "post" })

watch([() => props.sessionId, branchId, () => extension.value?.generation, () => panel.value?.id, ready, ended, () => connection.phase, attempt], () => dispose?.(), { flush: "sync" })

function close(): void {
  dispose?.()
  ui.closeDetail()
  void nextTick(() => document.querySelector<HTMLElement>("[data-extension-trigger]")?.focus())
}
function changeBranch(event: Event): void {
  ui.activeLineTab = { ...ui.activeLineTab, [props.sessionId]: (event.target as HTMLSelectElement).value }
}
async function retry(): Promise<void> {
  await extensions.refresh(true)
  attempt.value++
}
onBeforeUnmount(() => dispose?.())
</script>

<template>
  <component :is="ui.shellMode === 'compact' ? 'dialog' : 'section'" ref="surface" class="extension-panel" :aria-label="title" tabindex="-1" @cancel.prevent="close" @keydown.stop @pointerdown.stop="ui.detailFocused = true" @focusin="ui.detailFocused = true">
    <header class="extension-header">
      <img v-if="panel?.icon && extension" :src="resourceUrl(extension.generation, panel.icon)" alt="" width="20" height="20">
      <strong>{{ title }}</strong>
      <button type="button" class="ui-button" data-variant="ghost" data-size="sm" @click="ui.openLines(sessionId)">{{ t("lines") }}</button>
      <button type="button" class="ui-button" data-variant="ghost" data-size="sm" :aria-label="t('close')" @click="close">×</button>
    </header>
    <label class="extension-branch">{{ t("branch") }}
      <select :value="branchId" @change="changeBranch"><option v-for="line in view?.lines" :key="line.id" :value="line.id">{{ line.purpose || line.id }}</option></select>
    </label>
    <div v-if="status" class="extension-status" role="status">{{ status }} <button v-if="failure || !panel" type="button" class="ui-button" data-size="sm" @click="retry">{{ t("retry") }}</button></div>
    <div ref="container" class="extension-root" :aria-label="title" tabindex="-1" :hidden="Boolean(status)" />
  </component>
</template>

<style scoped>
.extension-panel { display: flex; flex-direction: column; height: 100%; min-height: 0; width: 100%; color: var(--foreground); background: var(--background); padding: 0; border: 0; }
dialog.extension-panel { max-width: none; max-height: none; height: 100dvh; margin: 0; }
.extension-header { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-3); border-bottom: 1px solid var(--border); }
.extension-header strong { flex: 1; }
.extension-branch { display: flex; gap: var(--space-2); padding: var(--space-2); }
.extension-branch select { min-width: 0; flex: 1; }
.extension-status { padding: var(--space-3); }
.extension-root { flex: 1; min-height: 0; overflow: auto; --dim-ui-background: var(--background); --dim-ui-foreground: var(--foreground); --dim-ui-muted: var(--muted-foreground); --dim-ui-border: var(--border); --dim-ui-accent: var(--primary); --dim-ui-font-family: var(--font-sans); }
.extension-root[hidden] { display: none; }
</style>
