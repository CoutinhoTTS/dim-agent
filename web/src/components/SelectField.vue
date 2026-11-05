<script setup lang="ts" generic="T">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from "vue"
import { Check, ChevronDown, Search } from "lucide-vue-next"
import { useSettingsText } from "../i18n/settings-general"

const props = defineProps<{ modelValue: T; options: { value: T; label: string; disabled?: boolean }[]; label: string; searchable?: boolean; creatable?: boolean; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: T]; create: [value: string] }>()
const s = useSettingsText()
const id = useId()
const trigger = ref<HTMLButtonElement>()
const panel = ref<HTMLDivElement>()
const search = ref<HTMLInputElement>()
const opened = ref(false)
const query = ref("")
const active = ref(0)
const position = ref({ left: "0px", top: "0px", width: "0px" })
const selected = computed(() => props.options.find(option => option.value === props.modelValue))
const filtered = computed(() => props.options.filter(option => option.label.toLocaleLowerCase().includes(query.value.toLocaleLowerCase())))
const choices = computed(() => filtered.value.flatMap((option, index) => option.disabled ? [] : [index]))
const canCreate = computed(() => props.creatable && query.value.trim() && !props.options.some(option => option.value === query.value.trim()))
function close(): void { panel.value?.hidePopover(); opened.value = false }
function scrolled(event: Event): void { if (!(event.target instanceof Node) || !panel.value?.contains(event.target)) void reposition() }
async function reposition(): Promise<void> {
  if (!opened.value || !trigger.value || !panel.value) return
  const rect = trigger.value.getBoundingClientRect()
  if (rect.bottom < 0 || rect.top > window.innerHeight) { close(); return }
  const width = Math.min(Math.max(rect.width, 200), window.innerWidth - 16)
  position.value = { left: `${Math.max(8, Math.min(rect.left, window.innerWidth - width - 8))}px`, top: `${rect.bottom + 4}px`, width: `${width}px` }
  await nextTick()
  const height = panel.value.getBoundingClientRect().height
  position.value.top = `${rect.bottom + height + 8 < window.innerHeight ? rect.bottom + 4 : Math.max(8, rect.top - height - 4)}px`
}
async function open(): Promise<void> {
  if (props.disabled || trigger.value?.matches(":disabled")) return
  if (opened.value) { close(); return }
  query.value = ""
  active.value = selected.value && !selected.value.disabled ? props.options.indexOf(selected.value) : choices.value[0] ?? 0
  opened.value = true
  panel.value?.showPopover()
  await reposition()
  if (props.searchable) search.value?.focus()
  else trigger.value?.focus()
}
function choose(index: number): void {
  const option = filtered.value[index]
  if (!option || option.disabled) return
  emit("update:modelValue", option.value)
  close()
  trigger.value?.focus()
}
function create(): void {
  if (!canCreate.value) return
  emit("create", query.value.trim())
  close()
  trigger.value?.focus()
}
async function keydown(event: KeyboardEvent): Promise<void> {
  if (event.isComposing || event.keyCode === 229) return
  if (!opened.value && ["ArrowDown", "ArrowUp"].includes(event.key)) { event.preventDefault(); await open(); return }
  if (!opened.value) return
  if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    event.preventDefault()
    const index = choices.value.indexOf(active.value)
    const next = event.key === "Home" ? 0 : event.key === "End" ? choices.value.length - 1 : (index + (event.key === "ArrowDown" ? 1 : -1) + choices.value.length) % choices.value.length
    active.value = choices.value[next] ?? 0
    await nextTick()
    panel.value?.querySelector(`[id="${id}-${active.value}"]`)?.scrollIntoView({ block: "nearest" })
  } else if (event.key === "Enter" || (event.key === " " && !props.searchable)) {
    event.preventDefault()
    if (!filtered.value.length && canCreate.value) create()
    else choose(active.value)
  } else if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); trigger.value?.focus() }
  else if (event.key === "Tab") close()
}
watch(query, async () => { active.value = choices.value[0] ?? 0; await nextTick(); await reposition() }, { flush: "sync" })
watch(() => props.disabled, value => { if (value) close() })
onMounted(() => { window.addEventListener("resize", reposition); window.addEventListener("scroll", scrolled, true) })
onBeforeUnmount(() => { close(); window.removeEventListener("resize", reposition); window.removeEventListener("scroll", scrolled, true) })
</script>

<template>
  <div class="select-field">
    <button ref="trigger" type="button" role="combobox" class="ui-field select-trigger" data-size="md" :aria-label="label" :aria-expanded="opened" :aria-controls="id" aria-haspopup="listbox" :aria-activedescendant="opened && filtered.length ? `${id}-${active}` : undefined" :disabled="disabled" @click="open" @keydown="keydown"><span>{{ selected?.label ?? s('Choose…', '请选择…') }}</span><ChevronDown :size="14" aria-hidden="true" /></button>
    <div ref="panel" popover="auto" class="ui-popover select-panel" :style="position" @toggle="opened = $event.newState === 'open'" @keydown="keydown">
      <div v-if="searchable" class="ui-menu-search"><Search :size="14" aria-hidden="true" /><input ref="search" v-model="query" type="search" :aria-label="s('Search ', '搜索') + label" :placeholder="creatable ? s('Search or enter a host path', '搜索或输入主机路径') : s('Search…', '搜索…')" :aria-controls="id" :aria-activedescendant="filtered.length ? `${id}-${active}` : undefined" /></div>
      <div :id="id" class="select-options" role="listbox" :aria-label="label"><button v-for="(option, index) in filtered" :id="`${id}-${index}`" :key="index" type="button" role="option" tabindex="-1" class="ui-menu-item" :aria-selected="option === selected" :disabled="option.disabled" :data-highlighted="index === active ? '' : undefined" @pointermove="active = index" @click="choose(index)"><span>{{ option.label }}</span><Check v-if="option === selected" :size="14" aria-hidden="true" /></button><span v-if="!filtered.length" class="select-empty">{{ s('No matches', '没有匹配项') }}</span></div>
      <button v-if="canCreate" class="ui-menu-item select-create" type="button" @click="create">{{ s('Use host path: ', '使用主机路径：') }}{{ query.trim() }}</button>
    </div>
  </div>
</template>

<style scoped>
.select-field { display: block; min-width: 0; width: 100%; }
.select-trigger { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); width: 100%; text-align: left; cursor: pointer; }
.select-trigger > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.select-trigger > svg { flex: none; }
.select-panel { position: fixed; inset: auto; margin: 0; padding: var(--space-1); min-width: 0; max-width: none; max-height: min(360px, calc(100dvh - 16px)); }
.select-panel:popover-open { display: flex; flex-direction: column; }
.select-options { display: block; min-height: 0; overflow-y: auto; }
.select-options > button { width: 100%; justify-content: space-between; min-height: 36px; white-space: normal; overflow-wrap: anywhere; }
.select-options > button > span { min-width: 0; white-space: normal; }
.select-options > button > svg { flex: none; }
.select-options > button:disabled { opacity: .5; }
.select-empty { display: block; padding: var(--space-3); color: var(--muted-foreground); font-size: var(--text-md); }
.select-create { width: 100%; border-top: 1px solid var(--border); white-space: normal; overflow-wrap: anywhere; }
@media (max-width: 719px) { .select-trigger, .select-options > button, .select-create { min-height: 44px; } }
</style>
