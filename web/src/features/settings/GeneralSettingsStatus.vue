<script setup lang="ts">
import { useSettingsText } from "../../i18n/settings-general"
defineProps<{ busy: boolean; error: string; saved?: boolean }>()
defineEmits<{ reload: [] }>()
const s = useSettingsText()
</script>

<template>
  <p v-if="busy" class="ui-banner settings-banner" role="status">{{ s('Working…', '正在处理…') }}</p>
  <p v-if="error" class="ui-banner settings-banner" data-tone="danger" role="alert">{{ error }} <button class="ui-button" data-size="sm" data-variant="secondary" type="button" :disabled="busy" @click="$emit('reload')">{{ s('Reload', '重新加载') }}</button></p>
  <p v-if="saved && !error" class="ui-banner settings-banner" data-tone="success" role="status">{{ s('Saved', '已保存') }}</p>
</template>
