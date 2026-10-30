<script setup lang="ts">
import { computed, ref, watch } from "vue"
import { RouterLink } from "vue-router"
import { useI18n } from "vue-i18n"
import { GitBranch, Menu } from "lucide-vue-next"
import BrandMark from "../../components/BrandMark.vue"
import ApprovalPrompt from "./ApprovalPrompt.vue"
import ComposerPanel from "./ComposerPanel.vue"
import TimelineView from "./TimelineView.vue"
import { MAIN_BRANCH, type ModelId } from "../../rpc/protocol"
import { useChatStore } from "../../stores/chat"
import { useConnectionStore } from "../../stores/connection"
import { useSessionsStore, workspaceName } from "../../stores/sessions"
import { useUiStore } from "../../stores/ui"

const props = defineProps<{ sessionId: string }>()
const { t } = useI18n()
const chat = useChatStore()
const connection = useConnectionStore()
const sessions = useSessionsStore()
const ui = useUiStore()

watch(() => props.sessionId, (id) => chat.open(id), { immediate: true })

const view = computed(() => chat.views[props.sessionId])
const summary = computed(() => sessions.byId.get(props.sessionId))
const title = computed(() => summary.value?.preview ?? t("sidebar.untitled"))
const approval = computed(() => view.value?.approvals.find((request) => request.branchId === MAIN_BRANCH))
const approvalKeyboard = computed(() => ui.approvalKeyboard(props.sessionId, MAIN_BRANCH, view.value?.approvals ?? []))
const composer = ref<InstanceType<typeof ComposerPanel>>()
const lineCount = computed(() => view.value?.lines.filter((line) => line.id !== MAIN_BRANCH).length ?? 0)
const linesBusy = computed(() => {
  const approvals = view.value?.approvals ?? []
  return view.value?.lines.some((line) => line.id !== MAIN_BRANCH && (line.state.kind === "working" || approvals.some((request) => request.branchId === line.id))) ?? false
})
const branchable = computed(() => connection.supports("session.branch_end") && view.value?.status === "ready")
const canNewLine = computed(() => connection.supports("session.branch_create") && view.value?.status === "ready")

function send(text: string): Promise<void> {
  return chat.submit(props.sessionId, MAIN_BRANCH, text)
}

function stop(): Promise<void> {
  return chat.stop(props.sessionId, MAIN_BRANCH)
}

function selectModel(model: ModelId): Promise<void> {
  return chat.setModel(props.sessionId, model)
}

function openLine(branchId: string): void {
  chat.ensureLine(props.sessionId, branchId)
  ui.openLine(props.sessionId, branchId)
}

function branch(text: string, cut: number): void {
  ui.requestNewLine({ sessionId: props.sessionId, parentBranchId: MAIN_BRANCH, at: cut, text })
}

async function createBranch(prompt: string): Promise<void> {
  const at = view.value?.lastTurnEnd
  if (at === undefined) throw new Error(t("lines.noFinishedTurn"))
  const branchId = await chat.createLine(props.sessionId, MAIN_BRANCH, at, prompt)
  ui.openLine(props.sessionId, branchId)
}
</script>

<template>
  <section v-if="view?.status === 'closed'" class="removed-state route-page">
    <h1 data-route-heading tabindex="-1">{{ t("chat.removedTitle") }}</h1>
    <p>{{ t("chat.removedDescription") }}</p>
    <RouterLink to="/chat/new">{{ t("sidebar.newSession") }}</RouterLink>
  </section>

  <section v-else-if="view?.status === 'failed'" class="removed-state route-page">
    <h1 data-route-heading tabindex="-1">{{ title }}</h1>
    <p role="alert">{{ view.error }}</p>
  </section>

  <section v-else-if="!view || view.status === 'loading'" class="chat-loading" aria-busy="true">
    <h1 class="sr-only">{{ t("route.session") }}</h1>
    <BrandMark animated class="boot-mark" />
    <p>{{ t("common.loading") }}</p>
  </section>

  <section v-else class="chat-page">
    <div class="chat-page-top">
      <header class="session-header">
        <button v-if="ui.shellMode === 'compact'" class="ui-icon-button session-nav" data-variant="ghost" data-size="md" type="button" :aria-label="t('sidebar.openMenu')" @click="ui.drawerOpen = true"><Menu aria-hidden="true" /></button>
        <div class="session-heading">
          <template v-if="summary">
            <span class="workspace-crumb" :title="summary.cwd">{{ workspaceName(summary.cwd) }}</span>
            <span class="workspace-crumb-separator" aria-hidden="true">/</span>
          </template>
          <h1 data-route-heading tabindex="-1" :title="title">{{ title }}</h1>
        </div>
        <button
          v-if="connection.supports('session.branch_end')"
          class="ui-button session-lines"
          data-variant="ghost"
          data-size="sm"
          type="button"
          :aria-label="t('lines.open')"
          @click="ui.toggleLines(sessionId)"
        >
          <GitBranch aria-hidden="true" />
          <span>{{ t("lines.title") }}</span>
          <span v-if="lineCount > 0" class="ui-badge" data-size="sm" :data-tone="linesBusy ? 'warning' : undefined">{{ lineCount }}</span>
        </button>
        <span v-if="view.sessionRunning" class="ui-badge session-running" data-tone="primary"><span class="status-dot" data-state="running" aria-hidden="true" />{{ t(view.stopping ? "status.stopping" : "status.running") }}</span>
      </header>
    </div>
    <TimelineView
      :rows="view.rows"
      :running="view.running"
      :quotable="!approval"
      :branchable="branchable && !approval"
      @branch="branch"
      @open-line="openLine"
      @quote="composer?.quote($event)"
    />
    <div v-if="approval" class="composer-panel">
      <ApprovalPrompt :key="JSON.stringify([approval.branchId, approval.requestId])" :session-id="sessionId" :request="approval" :keyboard="approvalKeyboard" />
    </div>
    <ComposerPanel
      v-else
      ref="composer"
      :running="view.running"
      :stopping="view.stopping"
      :model="view.model"
      :send="send"
      :stop="stop"
      :model-selectable="connection.supports('session.model.update')"
      :select-model="selectModel"
      :create-branch="canNewLine ? createBranch : undefined"
      :open-lines="branchable ? () => ui.openLines(sessionId) : undefined"
    />
  </section>
</template>

<style scoped>
.session-running { gap: 6px; }
.session-running > .status-dot { border: 0; }
.session-lines { gap: 6px; flex: none; }
.session-lines .ui-badge { min-width: 18px; padding-inline: 4px; }
</style>
