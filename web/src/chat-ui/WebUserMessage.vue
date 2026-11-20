<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue"
import { useI18n } from "vue-i18n"
import {
  TimelineUserActions,
  TimelineUserBubble,
  TimelineUserContent,
  getLinkIconKind,
  type ChatUiTimelineRowMessage,
} from "@dimcode/chat-ui"
import { copyText } from "../clipboard"
import { formatTimelineTimestamp } from "../features/chat/timeline-time"
import { useToastStore } from "../stores/toast"
import { Button, Empty, Passthrough, fileIconUrl, icon, icons, useWebTimeline } from "./primitives"

const props = defineProps<{ message: ChatUiTimelineRowMessage }>()
const { t, locale } = useI18n()
const toast = useToastStore()
const context = useWebTimeline()
const row = computed(() => {
  const source = context.rowByKey(props.message.id)
  return source?.kind === "user" ? source : undefined
})
const timeText = computed(() => formatTimelineTimestamp(props.message.createdAt, locale.value))
const copied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | undefined
onBeforeUnmount(() => clearTimeout(copiedTimer))

const contentIcons = { skill: icon("i-lucide-sparkles"), commit: icon("i-lucide-git-commit-horizontal"), genericLink: icon("i-lucide-link") }
const actionIcons = {
  withdraw: icon("i-lucide-undo-2"),
  copy: icons.copy,
  check: icons.check,
  edit: icon("i-lucide-pencil"),
  fork: icons.gitFork,
  branch: icons.gitBranch,
}

async function copy(): Promise<void> {
  try {
    await copyText(props.message.content)
  } catch {
    toast.push({ tone: "danger", title: t("chat.copyFailed") })
    return
  }
  copied.value = true
  clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => { copied.value = false }, 1500)
}
</script>

<template>
  <article class="web-user-message" data-role="user">
    <strong class="sr-only">{{ t("chat.you") }}</strong>
    <div v-if="message.attachments?.length" class="flex flex-wrap justify-end gap-2">
      <img v-for="attachment in message.attachments" :key="attachment.id" :src="attachment.url" :alt="t('chatTimeline.imageAttachment')" loading="lazy" class="max-h-64 max-w-full rounded-lg" />
    </div>
    <TimelineUserBubble :message-id="message.id" class="web-user-bubble">
      <TimelineUserContent
        :content="message.content"
        :icons="contentIcons"
        :resolve-file-icon-lookup-path="(path: string) => path"
        :get-file-icon-url="fileIconUrl"
        :normalize-attachment-reference="(value: string) => value"
        :is-pasted-text-name="() => false"
        :get-link-icon-kind="getLinkIconKind"
      />
    </TimelineUserBubble>
    <div v-for="skill in row?.skills ?? []" :key="skill" class="web-user-note">{{ t("message.skillLoaded", { name: skill }) }}</div>
    <div v-for="hook in row?.hooks ?? 0" :key="hook" class="web-user-note">{{ t("message.hookContext") }}</div>
    <TimelineUserActions
      :time-text="timeText"
      :can-withdraw="false"
      :can-edit="false"
      :can-fork="false"
      :can-branch="false"
      :copied="copied"
      :labels="context.labels.value.user"
      :icons="actionIcons"
      :button-component="Button"
      :tooltip-provider-component="Passthrough"
      :tooltip-component="Passthrough"
      :tooltip-trigger-component="Passthrough"
      :tooltip-content-component="Empty"
      @copy="copy"
    />
  </article>
</template>

<style scoped>
.web-user-message { display: flex; min-width: 0; flex-direction: column; align-items: flex-end; gap: var(--space-1); margin-bottom: var(--space-2); }
.web-user-bubble { max-width: 85%; white-space: pre-wrap; overflow-wrap: anywhere; }
.web-user-note { color: var(--muted-foreground); font-size: var(--text-xs); }
</style>
