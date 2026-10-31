<script setup lang="ts">
import { onBeforeUnmount, ref, useId } from "vue"
import { useI18n } from "vue-i18n"
import { TriangleAlert } from "lucide-vue-next"
import { settingsMessages } from "../i18n/settings"

const { t } = useI18n({ messages: settingsMessages })
const id = useId()
const dialog = ref<HTMLDialogElement>()
const message = ref("")
const label = ref("")
let settle: ((value: boolean) => void) | undefined

function open(text: string, options: { confirmLabel?: string } = {}): Promise<boolean> {
  settle?.(false)
  message.value = text
  label.value = options.confirmLabel ?? t("confirm")
  return new Promise((resolve) => {
    settle = resolve
    dialog.value?.showModal()
  })
}
function finish(value: boolean): void {
  const resolve = settle
  settle = undefined
  dialog.value?.close()
  resolve?.(value)
}
onBeforeUnmount(() => settle?.(false))
defineExpose({ open })
</script>

<template>
  <dialog ref="dialog" class="ui-dialog" data-size="md" :aria-labelledby="id" @cancel.prevent="finish(false)" @close="finish(false)">
    <header class="ui-dialog-header">
      <span class="ui-dialog-icon" data-tone="warning"><TriangleAlert aria-hidden="true" /></span>
      <h2 :id="id" class="ui-dialog-title">{{ message }}</h2>
    </header>
    <footer class="ui-dialog-footer">
      <button class="ui-button" data-variant="secondary" type="button" autofocus @click="finish(false)">{{ t("cancel") }}</button>
      <button class="ui-button" data-variant="danger" type="button" @click="finish(true)">{{ label }}</button>
    </footer>
  </dialog>
</template>
