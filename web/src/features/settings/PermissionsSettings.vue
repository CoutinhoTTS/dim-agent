<script setup lang="ts">
import { computed, onMounted, ref } from "vue"
import { useSettingsText } from "../../i18n/settings-general"
import { readRevision, readRules, type CommandRule, type RulesView } from "../../rpc/general-settings"
import { readIsolation, type IsolationView } from "../../rpc/management"
import { useConnectionStore } from "../../stores/connection"
import GeneralSettingsStatus from "./GeneralSettingsStatus.vue"
import { bindSettingsDirty, messageFrom } from "./context"

const s = useSettingsText()
const connection = useConnectionStore()
const snapshot = ref<RulesView>()
const rules = ref<CommandRule[]>([])
const busy = ref(false)
const error = ref("")
const saved = ref(false)
const isolation = ref<IsolationView>()
const sandbox = ref<IsolationView["effective"]["sandbox"]>("workspace")
const escalation = ref<IsolationView["effective"]["escalation"]>("ask")
const isolationError = ref("")
const isolationDirty = computed(() => isolation.value !== undefined && (sandbox.value !== isolation.value.effective.sandbox || escalation.value !== isolation.value.effective.escalation))
const rulesDirty = computed(() => snapshot.value !== undefined && JSON.stringify(rules.value) !== JSON.stringify(snapshot.value.commandRules))
const dirty = computed(() => rulesDirty.value || isolationDirty.value)
bindSettingsDirty(dirty)

async function load(): Promise<void> {
  if (busy.value) return
  if (dirty.value && !window.confirm(s('Discard unsaved command rules?', '放弃未保存的命令规则？'))) return
  busy.value = true
  error.value = ""
  saved.value = false
  try {
    if (!connection.supports("permission.rules")) throw new Error(s('Command rules are not available on this server.', '此服务器不支持命令规则。'))
    snapshot.value = await connection.client.call("permission.rules", {}, readRules)
    rules.value = snapshot.value.commandRules.map((rule) => ({ ...rule }))
  } catch (cause) { error.value = messageFrom(cause) }
  try {
    isolationError.value = ""
    if (connection.supports("host.isolation.get")) {
      isolation.value = await connection.client.call("host.isolation.get", {}, readIsolation)
      sandbox.value = isolation.value.effective.sandbox
      escalation.value = isolation.value.effective.escalation
    }
  } catch (cause) { isolationError.value = messageFrom(cause) }
  busy.value = false
}

async function save(): Promise<void> {
  if (!snapshot.value || !rulesDirty.value || busy.value) return
  if (rules.value.some((rule) => !rule.command.trim() || /[;&|`()<>$\n\r]/.test(rule.command))) {
    error.value = s('Enter a simple command without shell separators, substitutions or redirection.', '请输入不含 Shell 分隔符、替换或重定向的简单命令。')
    return
  }
  if (rules.value.some((rule) => rule.decision === "allow") && !window.confirm(s('Save command rules that allow execution without asking? Deny presets still take precedence.', '保存允许免询问执行的命令规则？拒绝预设仍然优先。'))) return
  busy.value = true
  error.value = ""
  saved.value = false
  try {
    const revision = await connection.client.call("permission.set_rules", { expectedRevision: snapshot.value.revision, commandRules: rules.value }, readRevision)
    snapshot.value = { revision, commandRules: rules.value.map((rule) => ({ ...rule })) }
    saved.value = true
  } catch (cause) { error.value = messageFrom(cause) }
  finally { busy.value = false }
}
async function saveIsolation(reset = false): Promise<void> {
  if (!isolation.value || busy.value || (!reset && !isolationDirty.value)) return
  const tiers = { workspace: 0, network: 1, host: 2 }
  const increasesAccess = tiers[sandbox.value] > tiers[isolation.value.effective.sandbox] || (escalation.value === "ask" && isolation.value.effective.escalation === "deny")
  if ((reset || increasesAccess) && !window.confirm(reset ? s('Reset command isolation to workspace-only access with approval for escalation?', '将命令隔离重置为仅工作区访问、提权需审批？') : s('Increase command access on the CLI host? Network allows outbound connections; host removes process isolation. Running processes are unchanged.', '扩大 CLI 主机上的命令访问范围？网络模式允许出站连接，主机模式取消进程隔离。已运行的进程不受影响。'))) return
  busy.value = true
  isolationError.value = ""
  try {
    isolation.value = await connection.client.call(reset ? "host.isolation.reset" : "host.isolation.update", {
      expectedRevision: isolation.value.revision,
      ...(!reset ? { policy: { version: 2, sandbox: sandbox.value, escalation: escalation.value, ...isolation.value.policy } } : {}),
    }, readIsolation)
    sandbox.value = isolation.value.effective.sandbox
    escalation.value = isolation.value.effective.escalation
  } catch (cause) { isolationError.value = messageFrom(cause) }
  finally { busy.value = false }
}
onMounted(load)
</script>

<template>
  <article class="settings-page">
    <header><h2 data-route-heading tabindex="-1">{{ s('Permissions', '权限') }}</h2><p>{{ s('User-level command rules apply to process-starting agent tools in every session. Deny wins; Allow never overrides a denied permission preset.', '用户命令规则适用于所有会话中启动进程的代理工具。拒绝优先；允许规则不会越过权限预设的拒绝。') }}</p></header>
    <GeneralSettingsStatus :busy="busy" :error="error" :saved="saved && !dirty" @reload="load" />
    <p v-if="isolationError" class="ui-banner settings-banner" data-tone="danger" role="alert">{{ isolationError }} <button class="ui-button" data-variant="ghost" :disabled="busy" @click="load">{{ s('Reload', '重新加载') }}</button></p>
    <form v-if="isolation" class="settings-card settings-form" @submit.prevent="saveIsolation()">
      <h3>{{ s('Command isolation', '命令隔离') }}</h3>
      <p>{{ s('Applies to new processes on this execution host. Existing processes keep their original boundary. Existing file and environment grants are preserved.', '适用于此执行主机上的新进程。已运行的进程保留原隔离边界。已有文件和环境变量授权保持不变。') }}</p>
      <fieldset class="settings-form" :disabled="busy || !connection.supports('host.isolation.update')">
        <label><span>{{ s('Execution and network access', '执行与网络访问') }}</span><select v-model="sandbox" class="ui-field"><option value="workspace">{{ s('Workspace — isolated, no network', '工作区 — 隔离且无网络') }}</option><option value="network">{{ s('Network — isolated, with network', '网络 — 隔离且允许联网') }}</option><option value="host">{{ s('Host — no process isolation', '主机 — 无进程隔离') }}</option></select></label>
        <label><span>{{ s('Escalation requests', '提权请求') }}</span><select v-model="escalation" class="ui-field"><option value="deny">{{ s('Deny', '拒绝') }}</option><option value="ask">{{ s('Ask for approval', '请求审批') }}</option></select></label>
      </fieldset>
      <div class="settings-actions"><button class="ui-button" data-variant="secondary" type="button" :disabled="busy || !connection.supports('host.isolation.reset')" @click="saveIsolation(true)">{{ s('Reset isolation', '重置隔离') }}</button><button class="ui-button" data-variant="primary" :disabled="busy || !isolationDirty || !connection.supports('host.isolation.update')">{{ s('Save isolation', '保存隔离设置') }}</button></div>
    </form>
    <form v-if="snapshot" class="settings-form" @submit.prevent="save">
      <section class="settings-card">
        <h3>{{ s('Command rules', '命令规则') }}</h3>
        <fieldset :disabled="busy || !connection.supports('permission.set_rules')">
          <legend>{{ s('Match commands by words, not wildcards or regular expressions.', '按单词匹配命令，不支持通配符或正则表达式。') }}</legend>
          <div v-for="(rule, index) in rules" :key="index" class="rule-row">
            <input v-model="rule.command" class="ui-field" data-size="md" :aria-label="s('Command', '命令')" placeholder="git status" required />
            <select v-model="rule.match" class="ui-field" data-size="md" :aria-label="s('Match', '匹配方式')"><option value="exact">{{ s('Exact', '精确') }}</option><option value="prefix">{{ s('Prefix', '前缀') }}</option></select>
            <select v-model="rule.decision" class="ui-field" data-size="md" :aria-label="s('Decision', '决策')"><option value="allow">{{ s('Allow', '允许') }}</option><option value="deny">{{ s('Deny', '拒绝') }}</option></select>
            <button class="ui-button" data-size="md" data-variant="danger-ghost" type="button" @click="rules.splice(index, 1)">{{ s('Remove', '移除') }}</button>
          </div>
          <p v-if="!rules.length">{{ s('No command rules. The session permission preset and approval flow decide each call.', '尚无命令规则。每次调用由会话权限预设和审批流程决定。') }}</p>
          <button class="ui-button" data-size="md" data-variant="secondary" type="button" @click="rules.push({ command: '', match: 'prefix', decision: 'deny' })">{{ s('Add rule', '添加规则') }}</button>
        </fieldset>
      </section>
      <footer class="settings-actions"><button class="ui-button" data-size="md" data-variant="secondary" type="button" :disabled="busy" @click="load">{{ s('Reload', '重新加载') }}</button><button class="ui-button" data-size="md" data-variant="primary" :disabled="busy || !rulesDirty || !connection.supports('permission.set_rules')">{{ s('Save rules', '保存规则') }}</button></footer>
    </form>
  </article>
</template>

<style scoped>
fieldset { min-width: 0; padding: 0; border: 0; }
label { display: grid; gap: var(--space-2); }
legend { margin-bottom: var(--space-4); color: var(--muted-foreground); font-size: var(--text-sm); }
.rule-row { display: grid; grid-template-columns: minmax(120px, 1fr) auto auto auto; gap: var(--space-2); margin-bottom: var(--space-3); }
@media (max-width: 640px) { .rule-row { grid-template-columns: 1fr 1fr; } .rule-row input { grid-column: 1 / -1; } }
</style>
