<script setup lang="ts">
import { computed } from "vue"
import { useI18n } from "vue-i18n"
import { useExtensionsStore } from "../../stores/extensions"
import { useUiStore } from "../../stores/ui"
import { panelTitle } from "./context"
import { messages } from "./messages"

defineProps<{ sessionId?: string }>()
const { t, locale } = useI18n({ messages })
const extensions = useExtensionsStore()
const ui = useUiStore()
const panels = computed(() => (extensions.directory?.extensions ?? []).flatMap(extension => extension.panels.map(panel => ({ extension, panel }))).sort((a, b) => a.extension.pluginName.localeCompare(b.extension.pluginName) || a.panel.id.localeCompare(b.panel.id)))
function select(event: Event): void {
  const value = (event.target as HTMLSelectElement).value
  const entry = panels.value.find(entry => JSON.stringify([entry.extension.pluginName, entry.panel.id]) === value)
  if (entry === undefined) return
  ui.extension = { pluginName: entry.extension.pluginName, panelId: entry.panel.id }
  ui.detailExpanded = false
  ui.detailOpen = true
  ui.detailFocused = true
}
</script>

<template>
  <label class="extension-picker">
    <span class="sr-only">{{ t("extensions") }}</span>
    <select data-extension-trigger :aria-label="t('extensions')" :title="sessionId ? t('choose') : t('session')" :disabled="!sessionId" :value="ui.extension && ui.detailOpen ? JSON.stringify([ui.extension.pluginName, ui.extension.panelId]) : ''" @change="select">
      <option value="">{{ panels.length ? t("choose") : t("empty") }}</option>
      <option v-for="{ extension, panel } in panels" :key="JSON.stringify([extension.pluginName, panel.id])" :value="JSON.stringify([extension.pluginName, panel.id])" :disabled="panel.placement !== 'session-panel'">{{ panelTitle(panel, locale) }} · {{ extension.pluginName }}</option>
      <option v-for="error in extensions.directory?.errors" :key="error.pluginName" disabled>{{ error.pluginName }}: {{ error.message }}</option>
    </select>
    <span v-if="extensions.error" role="status">{{ extensions.error }}</span>
  </label>
</template>

<style scoped>
.extension-picker { min-width: 0; flex: 0 1 auto; }
.extension-picker select { width: clamp(8rem, 18vw, 16rem); max-width: 100%; text-overflow: ellipsis; }
</style>
