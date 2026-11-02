<script setup lang="ts">
import { ref } from "vue"
import { useI18n } from "vue-i18n"
import { Check, Shield, ShieldCheck, Zap } from "lucide-vue-next"
import ConfirmDialog from "../../components/ConfirmDialog.vue"
import type { PermissionPreset } from "../../rpc/protocol"

/**
 * The session's permission preset, changed from the composer as dim-agent's
 * tool permission selector does: three presets with what each one allows, and
 * a confirmation before full access lifts the workspace limit.
 */
const props = defineProps<{ preset: PermissionPreset | undefined }>()
const emit = defineEmits<{ select: [preset: PermissionPreset] }>()
const { t } = useI18n()
const details = ref<HTMLDetailsElement>()
const confirm = ref<InstanceType<typeof ConfirmDialog>>()

const OPTIONS = [
  { preset: "read-only", icon: Shield, label: "composer.permission.readOnly", description: "composer.permission.readOnlyDescription" },
  { preset: "workspace-write", icon: ShieldCheck, label: "composer.permission.workspaceWrite", description: "composer.permission.workspaceWriteDescription" },
  { preset: "full-access", icon: Zap, label: "composer.permission.fullAccess", description: "composer.permission.fullAccessDescription" },
] as const

function current() {
  return OPTIONS.find((option) => option.preset === props.preset)
}

async function choose(preset: PermissionPreset): Promise<void> {
  details.value?.removeAttribute("open")
  if (preset === "full-access" && props.preset !== "full-access") {
    if (!await confirm.value?.open(t("composer.permission.fullAccessConfirm"), { confirmLabel: t("composer.permission.fullAccess") })) return
  }
  emit("select", preset)
}

function open(): void {
  if (details.value) details.value.open = true
}

defineExpose({ open })
</script>

<template>
  <details ref="details" class="composer-popover permission-menu">
    <summary class="ui-button permission-trigger" data-variant="ghost" data-size="sm" :aria-label="t('composer.permission.label')">
      <component :is="current()?.icon ?? Shield" :size="14" aria-hidden="true" />
      <span class="permission-trigger-label">{{ current() ? t(current()!.label) : t("composer.permission.label") }}</span>
    </summary>
    <div class="composer-popover-content ui-popover permission-content" data-width="lg">
      <button
        v-for="option in OPTIONS"
        :key="option.preset"
        type="button"
        class="ui-menu-item permission-option"
        :aria-current="option.preset === preset"
        @click="choose(option.preset)"
      >
        <component :is="option.icon" :size="14" aria-hidden="true" />
        <span class="permission-option-text">
          <span>{{ t(option.label) }}</span>
          <small>{{ t(option.description) }}</small>
        </span>
        <Check v-if="option.preset === preset" :size="14" aria-hidden="true" />
      </button>
    </div>
    <ConfirmDialog ref="confirm" />
  </details>
</template>

<style scoped>
.permission-trigger { gap: var(--space-1); }
.permission-content { display: grid; gap: var(--space-1); }
.permission-option { display: flex; width: 100%; align-items: flex-start; gap: var(--space-2); text-align: left; }
.permission-option-text { display: grid; min-width: 0; flex: 1; gap: 2px; }
.permission-option small { color: var(--muted-foreground); font-size: var(--text-xs); }
</style>
