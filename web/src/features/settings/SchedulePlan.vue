<script setup lang="ts">
import { computed, ref } from "vue"
import SelectField from "../../components/SelectField.vue"
import { useSettingsText } from "../../i18n/settings-general"
import type { Schedule } from "../../rpc/scheduler"

const props = defineProps<{ modelValue: Schedule }>()
const emit = defineEmits<{ 'update:modelValue': [value: Schedule]; valid: [value: boolean] }>()
const s = useSettingsText()
const initial = props.modelValue
const mode = ref("at" in initial ? "once" : "repeat")
const simple = "cron" in initial ? /^(\d{1,2}) (\d{1,2}) \* \* (\*|[0-6](?:,[0-6])*)$/.exec(initial.cron) : null
const advanced = ref("every_minutes" in initial ? "interval" : "cron" in initial && !simple ? "cron" : "basic")
const frequency = ref(simple && simple[3] !== "*" ? "weekly" : "daily")
const days = ref(simple && simple[3] !== "*" ? simple[3]!.split(",").map(Number) : [1, 2, 3, 4, 5])
const time = ref(simple ? `${simple[2]!.padStart(2, "0")}:${simple[1]!.padStart(2, "0")}` : "09:00")
const timezone = ref("cron" in initial ? initial.tz ?? "" : Intl.DateTimeFormat().resolvedOptions().timeZone)
const minutes = ref("every_minutes" in initial ? initial.every_minutes : 60)
const cron = ref("cron" in initial ? initial.cron : "0 9 * * 1-5")
const localZone = Intl.DateTimeFormat().resolvedOptions().timeZone
function localMinute(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0")
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}
const once = ref("at" in initial ? localMinute(new Date(initial.at)) : "")
const explicitInstant = ref("")
const error = ref("")
const ambiguous = ref(false)
const weekdays = computed(() => [s("Mon", "周一"), s("Tue", "周二"), s("Wed", "周三"), s("Thu", "周四"), s("Fri", "周五"), s("Sat", "周六"), s("Sun", "周日")])
const summary = computed(() => {
  const plan = props.modelValue
  if ("every_minutes" in plan) return s(`Every ${plan.every_minutes} minutes`, `每隔 ${plan.every_minutes} 分钟`)
  if ("at" in plan) return new Date(plan.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })
  return advanced.value === "basic" ? `${frequency.value === "daily" ? s("Every day", "每天") : s("Every selected weekday", "每周所选日期")} ${time.value}` : plan.cron
})
function update(): void {
  error.value = ""
  if (mode.value !== "once" || advanced.value !== "basic") ambiguous.value = false
  try {
    let value: Schedule
    if (advanced.value === "interval") {
      if (!Number.isInteger(minutes.value) || minutes.value < 1 || minutes.value > 4294967295) throw new Error(s("Enter an integer interval of at least 1 minute.", "请输入至少 1 分钟的整数间隔。"))
      value = { every_minutes: minutes.value }
    } else if (mode.value === "once" && advanced.value !== "cron") {
      let date = new Date(once.value)
      if (explicitInstant.value) {
        if (!/(Z|[+-]\d{2}:\d{2})$/.test(explicitInstant.value)) throw new Error(s("Include Z or an explicit UTC offset.", "请包含 Z 或明确的 UTC 偏移。"))
        date = new Date(explicitInstant.value)
      } else {
        if (!Number.isFinite(date.getTime()) || localMinute(date) !== once.value) throw new Error(s("This local minute does not exist. Choose another time.", "此本地时间不存在，请选择其他时间。"))
        ambiguous.value = false
        for (let offset = -180; offset <= 180; offset++) {
          if (offset !== 0 && localMinute(new Date(date.getTime() + offset * 60_000)) === once.value) ambiguous.value = true
        }
        if (ambiguous.value) throw new Error(s("This minute occurs twice. Enter an instant with its UTC offset below.", "此分钟在夏令时切换时出现两次，请在下方指定带 UTC 偏移的时间。"))
      }
      if (!Number.isFinite(date.getTime()) || date.getSeconds() !== 0 || date.getMilliseconds() !== 0) throw new Error(s("Choose a valid time at minute precision.", "请选择有效时间，精确到分钟。"))
      value = { at: date.toISOString() }
    } else {
      const [hour, minute] = time.value.split(":").map(Number)
      if (advanced.value === "basic" && (!/^\d{2}:\d{2}$/.test(time.value) || hour! > 23 || minute! > 59 || (frequency.value === "weekly" && !days.value.length))) throw new Error(s("Choose a time and at least one weekday.", "请选择运行时间和至少一个星期。"))
      const expression = advanced.value === "cron" ? cron.value.trim() : `${minute} ${hour} * * ${frequency.value === "daily" ? "*" : [...days.value].sort((a, b) => a - b).join(",")}`
      if (expression.split(/\s+/).length !== 5) throw new Error(s("Cron needs five fields: minute hour day month weekday.", "Cron 需要五个字段：分、时、日、月、周。"))
      if (timezone.value) new Intl.DateTimeFormat("en", { timeZone: timezone.value })
      value = { cron: expression, ...(timezone.value ? { tz: timezone.value } : {}) }
    }
    emit("update:modelValue", value)
    emit("valid", true)
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
    emit("valid", false)
  }
}
function selectMode(value: string): void { mode.value = value; advanced.value = "basic"; explicitInstant.value = ""; update() }
</script>

<template>
  <section class="settings-card settings-form">
    <h3>{{ s('When to run', '运行计划') }}</h3>
    <div class="settings-actions" :aria-label="s('Schedule type', '计划类型')">
      <button v-for="item in ['once', 'repeat']" :key="item" class="ui-button" data-size="md" :data-variant="mode === item && advanced === 'basic' ? 'secondary' : 'ghost'" type="button" :aria-pressed="mode === item && advanced === 'basic'" @click="selectMode(item)">{{ item === 'once' ? s('Once', '仅一次') : s('Repeat', '重复') }}</button>
    </div>
    <div v-if="advanced === 'basic'" class="settings-grid">
      <div v-if="mode === 'repeat'" class="schedule-control"><span>{{ s('Frequency', '重复频率') }}</span><SelectField v-model="frequency" :label="s('Frequency', '重复频率')" :options="[{ value: 'daily', label: s('Every day', '每天') }, { value: 'weekly', label: s('Every week', '每周') }]" @update:model-value="update" /></div>
      <label v-if="mode === 'repeat'"><span>{{ s('Run time', '运行时间') }}</span><input v-model="time" class="ui-field" data-size="md" type="time" step="60" required @input="update" /></label>
      <label v-else class="schedule-once"><span>{{ s('Date and time', '运行日期和时间') }}</span><input v-model="once" class="ui-field" data-size="md" type="datetime-local" step="60" required @input="explicitInstant = ''; update()" /></label>
    </div>
    <div v-if="advanced === 'basic' && mode === 'repeat' && frequency === 'weekly'" class="settings-actions"><label v-for="(label, index) in weekdays" :key="index"><input v-model="days" type="checkbox" :value="(index + 1) % 7" @change="update" />{{ label }}</label></div>
    <label v-if="advanced !== 'interval' && (mode === 'repeat' || advanced === 'cron')"><span>{{ s('Execution time zone', '执行时区') }}</span><input v-model="timezone" class="ui-field" data-size="md" placeholder="Asia/Shanghai" @change="update" /><small class="settings-meta">{{ s('Blank keeps the CLI host’s local time zone, not the browser’s.', '留空使用 CLI 主机本地时区，不是浏览器时区。') }}</small></label>
    <p v-if="mode === 'once' && advanced === 'basic'" class="settings-help">{{ s('Browser time zone: ', '浏览器时区：') }}{{ localZone }}. {{ s('Minute precision. A past instant may be recorded as missed.', '精确到分钟；过去时间可能被记为错过。') }}</p>
    <label v-if="ambiguous"><span>{{ s('Exact instant with UTC offset', '带 UTC 偏移的明确时间') }}</span><input v-model="explicitInstant" class="ui-field" data-size="md" placeholder="2026-11-01T01:30:00-05:00" @change="update" /></label>
    <details :open="advanced !== 'basic'">
      <summary>{{ s('Advanced schedule', '高级计划') }}</summary>
      <div class="settings-form">
        <div class="schedule-control"><span>{{ s('Plan', '计划') }}</span><SelectField v-model="advanced" :label="s('Plan', '计划')" :options="[{ value: 'basic', label: s('Use the date / frequency above', '使用上面的日期 / 重复频率') }, { value: 'interval', label: s('Every N minutes', '每隔 N 分钟') }, { value: 'cron', label: s('Custom Cron', '自定义 Cron') }]" @update:model-value="update" /></div>
        <template v-if="advanced === 'interval'"><label><span>{{ s('Interval in minutes', '间隔分钟数') }}</span><input v-model.number="minutes" class="ui-field" data-size="md" type="number" min="1" max="4294967295" step="1" required @input="update" /></label><p class="settings-help">{{ s('Anchored at enable time on the host, not a fixed clock time. Resuming resets the anchor. No time zone applies.', '从主机启用时间起计时，不保证固定钟点；恢复会重设起点，不受时区影响。') }}</p></template>
        <label v-if="advanced === 'cron'"><span>Cron · {{ s('minute hour day month weekday', '分 时 日 月 周') }}</span><input v-model="cron" class="ui-field" data-size="md" required @input="update" /></label>
      </div>
    </details>
    <p v-if="error" class="ui-field-hint" data-tone="danger" role="alert">{{ error }}</p>
    <p class="settings-help">{{ summary }}<br>{{ s('The host validates the plan and reports the next run after saving.', '主机校验计划，并在保存后返回下次执行时间。') }}</p>
  </section>
</template>

<style scoped>
.schedule-once { grid-column: 1 / -1; }
.schedule-control { display: grid; gap: var(--space-2); font-size: var(--text-md); }
.schedule-control > span { font-weight: var(--weight-medium); }
</style>
