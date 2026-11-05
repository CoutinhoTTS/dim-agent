<script setup lang="ts">
import { computed, onMounted, ref } from "vue"
import { RefreshCw } from "lucide-vue-next"
import { useSettingsText } from "../../i18n/settings-general"
import { readSkills, type SkillEntry, type SkillsView } from "../../rpc/general-settings"
import { readAck } from "../../rpc/read"
import { useConnectionStore } from "../../stores/connection"
import { useSessionsStore } from "../../stores/sessions"
import GeneralSettingsStatus from "./GeneralSettingsStatus.vue"
import { messageFrom, reloadSettingsOnReconnect } from "./context"

const s = useSettingsText()
const connection = useConnectionStore()
const sessions = useSessionsStore()
const catalog = ref<SkillsView>()
const cwd = ref("")
const search = ref("")
const busy = ref(false)
const error = ref("")
const status = ref("")
const entries = computed(() => catalog.value?.entries.filter((skill) => `${skill.name}\n${skill.description}\n${skill.filePath}`.toLocaleLowerCase().includes(search.value.toLocaleLowerCase())) ?? [])
function scope(path = cwd.value): Record<string, unknown> { return path ? { cwd: path } : {} }
async function readCatalog(path = cwd.value): Promise<void> {
  catalog.value = await connection.client.call("skills.catalog", scope(path), readSkills)
  cwd.value = path
}
async function load(refresh = false, path = cwd.value): Promise<void> {
  if (busy.value) return
  busy.value = true
  error.value = ""
  status.value = ""
  try {
    if (!connection.supports("skills.catalog")) throw new Error(s('Skills management is not available on this server.', '此服务器不支持技能管理。'))
    if (refresh) await connection.client.call("skills.refresh", scope(path), readAck)
    await readCatalog(path)
    if (refresh) status.value = s('Skills reloaded.', '技能已重新加载。')
  } catch (cause) { error.value = messageFrom(cause) }
  finally { busy.value = false }
}
async function toggle(skill: SkillEntry): Promise<void> {
  if (busy.value) return
  busy.value = true
  error.value = ""
  status.value = ""
  try {
    await connection.client.call("skills.set_enabled", { ...scope(), id: skill.id, enabled: !skill.enabled }, readAck)
    await readCatalog()
  } catch (cause) { error.value = messageFrom(cause) }
  finally { busy.value = false }
}
async function changeScope(event: Event): Promise<void> {
  const select = event.target as HTMLSelectElement
  await load(false, select.value)
  select.value = cwd.value
}
reloadSettingsOnReconnect(connection, () => load())
onMounted(() => load())
</script>

<template>
  <article class="settings-page">
    <header class="skill-heading">
      <div><h2 data-route-heading tabindex="-1">{{ s('Skills', '技能') }}</h2><p>{{ s('Manage the installed skills the agent may use.', '管理代理可使用的已安装技能。') }}</p></div>
      <button class="ui-button" data-size="md" data-variant="secondary" type="button" :disabled="busy || !connection.supports('skills.refresh')" @click="load(true)"><RefreshCw :size="14" aria-hidden="true" />{{ s('Reload skills', '重新加载技能') }}</button>
    </header>
    <div class="skill-toolbar">
      <label><span>{{ s('Scope', '范围') }}</span><select :value="cwd" class="ui-field" data-size="md" :disabled="busy" @change="changeScope"><option value="">{{ s('User skills', '用户技能') }}</option><option v-for="workspace in sessions.workspaces" :key="workspace.cwd" :value="workspace.cwd">{{ workspace.name }} — {{ workspace.cwd }}</option></select></label>
      <label><span>{{ s('Search skills', '搜索技能') }}</span><input v-model="search" class="ui-field" data-size="md" type="search" :placeholder="s('Name or description', '名称或描述')" /></label>
    </div>
    <GeneralSettingsStatus :busy="busy" :error="error" @reload="load()" />
    <p v-if="status" role="status" class="settings-banner">{{ status }}</p>
    <p v-for="issue in catalog?.issues" :key="`${issue.path}:${issue.message}`" class="ui-banner settings-banner" data-tone="warning"><code>{{ issue.path }}</code> — {{ issue.message }}</p>
    <ul v-if="entries.length" class="skill-list">
      <li v-for="skill in entries" :key="skill.filePath" class="skill-row">
        <div class="skill-heading">
          <h3>{{ skill.name }}</h3>
          <span class="ui-badge" :data-tone="skill.shadowedBy ? 'warning' : skill.enabled ? 'success' : undefined">{{ skill.shadowedBy ? s('Shadowed', '已被覆盖') : skill.enabled ? s('Enabled', '已启用') : s('Disabled', '已禁用') }}</span>
          <button class="ui-button" data-size="sm" data-variant="secondary" type="button" :aria-label="`${skill.enabled ? s('Disable', '禁用') : s('Enable', '启用')} ${skill.name}`" :disabled="busy || !!skill.shadowedBy || !connection.supports('skills.set_enabled')" @click="toggle(skill)">{{ skill.enabled ? s('Disable', '禁用') : s('Enable', '启用') }}</button>
        </div>
        <p>{{ skill.description }}</p>
        <p v-if="skill.shadowedBy" class="settings-meta">{{ s('Shadowed by', '被以下技能覆盖') }}: {{ skill.shadowedBy }}</p>
        <details><summary>{{ s('Details', '详情') }}</summary><code>{{ skill.filePath }}</code><p v-if="skill.allowedTools.length" class="settings-meta">{{ s('Declared tools', '声明的工具') }}: {{ skill.allowedTools.join(', ') }}</p></details>
      </li>
    </ul>
    <p v-else-if="catalog" role="status" class="settings-empty">{{ catalog.entries.length ? s('No matching skills.', '没有匹配的技能。') : s('No installed skills found. Add skills outside this page, then reload.', '未发现已安装的技能。在此页面之外添加技能后，重新加载。') }}</p>
  </article>
</template>

<style scoped>
.skill-heading { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); }
.skill-heading > div, .skill-heading h3 { min-width: 0; flex: 1; }
.skill-heading h3 { margin: 0; overflow-wrap: anywhere; }
.skill-heading > .ui-button { flex: none; }
.skill-toolbar { display: flex; flex-wrap: wrap; gap: var(--space-3); margin-bottom: var(--space-5); }
.skill-toolbar label { display: grid; flex: 1 1 180px; min-width: 0; gap: var(--space-2); font-size: var(--text-md); }
.skill-list { margin: 0; padding: 0; list-style: none; border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--card); }
.skill-row { padding: var(--space-4); }
.skill-row + .skill-row { border-top: 1px solid var(--border); }
.skill-row > p { margin: var(--space-2) 0; font-size: var(--text-md); color: var(--muted-foreground); overflow-wrap: anywhere; }
.skill-row details code { display: block; font-size: var(--text-sm); }
code { overflow-wrap: anywhere; }
</style>
