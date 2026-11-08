<script setup lang="ts">
import { computed, nextTick, reactive, ref } from "vue"
import { useI18n } from "vue-i18n"
import { CircleHelp } from "lucide-vue-next"
import type { AskQuestion, PendingQuestion, QuestionAnswer } from "../../rpc/protocol"
import { RpcError } from "../../rpc/client"
import { useChatStore } from "../../stores/chat"

/**
 * The `ask_user` prompt: it takes the composer's place and asks the questions
 * of one waiting call. "Send answers" answers the call, "Decline" answers it
 * with no answers. The prompt closes when the batch's result reaches the
 * timeline, so a rejected or late answer keeps the form for another try.
 */
const props = defineProps<{ sessionId: string; question: PendingQuestion }>()
const { t } = useI18n()
const chat = useChatStore()
const busy = ref(false)
const sent = ref(false)
const error = ref("")
const selected = reactive<Record<string, string[]>>({})
const typed = reactive<Record<string, string>>({})
const step = ref(0)
const body = ref<HTMLElement>()
const current = computed(() => props.question.questions[step.value])
const multiple = computed(() => props.question.questions.length > 1)
const radioGroup = (question: AskQuestion) => `question-${props.question.callId}-${question.id}`

function typedAnswer(question: AskQuestion): string | undefined {
  const text = (typed[question.id] ?? "").trim()
  return question.allowCustom && text !== "" ? text : undefined
}

function answered(question: AskQuestion): boolean {
  return (selected[question.id]?.length ?? 0) > 0 || typedAnswer(question) !== undefined
}
const complete = computed(() => props.question.questions.every(answered))

async function go(index: number): Promise<void> {
  step.value = index
  await nextTick()
  body.value?.scrollTo({ top: 0 })
  body.value?.focus()
}

function advance(): void {
  if (busy.value || sent.value) return
  if (current.value && multiple.value) {
    if (answered(current.value)) void go(step.value + 1)
  } else void send(false)
}

function choose(question: AskQuestion, label: string): void {
  const current = selected[question.id] ?? []
  if (!question.multiSelect) {
    selected[question.id] = [label]
    typed[question.id] = ""
  } else selected[question.id] = current.includes(label) ? current.filter((item) => item !== label) : [...current, label]
}

function typeAnswer(question: AskQuestion, text: string): void {
  typed[question.id] = text
  if (!question.multiSelect && text.trim() !== "") selected[question.id] = []
}

function answers(): QuestionAnswer[] {
  return props.question.questions.map((question) => {
    const custom = typedAnswer(question)
    return { questionId: question.id, selected: selected[question.id] ?? [], ...(custom === undefined ? {} : { custom }) }
  })
}

async function send(decline: boolean): Promise<void> {
  if (busy.value || sent.value || (!decline && !complete.value)) return
  busy.value = true
  error.value = ""
  try {
    await chat.answerQuestion(props.sessionId, props.question, decline ? undefined : answers())
    sent.value = true
  } catch (cause) {
    error.value = cause instanceof RpcError && cause.appError === "question_not_pending" ? t("question.notPending") : cause instanceof Error && cause.message ? cause.message : t("question.failed")
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <article class="prompt-card question-card" data-inline-interaction-prompt>
    <header class="prompt-header">
      <span class="prompt-icon" aria-hidden="true"><CircleHelp :size="12" :stroke-width="2.25" /></span>
      <h2 class="prompt-title">{{ t("question.title") }}</h2>
      <span class="ui-badge" data-tone="warning">{{ sent ? t("question.sent") : t("question.awaitingAnswer") }}</span>
    </header>

    <nav v-if="multiple" class="question-steps" :aria-label="t('question.steps')">
      <button v-for="(question, index) in props.question.questions" :key="question.id" type="button" class="ui-button" data-size="sm" data-variant="ghost" :aria-current="step === index ? 'step' : undefined" :disabled="busy || sent" @click="go(index)">{{ index + 1 }}. {{ question.header || t('question.number', { number: index + 1 }) }}<span v-if="answered(question)" aria-hidden="true"> ✓</span></button>
      <button type="button" class="ui-button" data-size="sm" data-variant="ghost" :aria-current="!current ? 'step' : undefined" :disabled="busy || sent" @click="go(props.question.questions.length)">{{ t('question.review') }}</button>
    </nav>

    <form class="question-form" @submit.prevent="advance">
      <div ref="body" class="question-body" tabindex="-1" :aria-label="current ? current.question : t('question.review')">
      <fieldset v-if="current" :key="current.id" class="question-item" :disabled="busy || sent">
        <legend>
          <span v-if="current.header" class="question-header">{{ current.header }}</span>
          {{ current.question }}
          <small v-if="current.multiSelect" class="question-hint">{{ t("question.selectMany") }}</small>
        </legend>
        <label v-for="option in current.options" :key="option.label" class="question-option">
          <input
            :type="current.multiSelect ? 'checkbox' : 'radio'"
            :name="radioGroup(current)"
            :checked="(selected[current.id] ?? []).includes(option.label)"
            @change="choose(current, option.label)"
          >
          <span class="question-option-text">
            <span>{{ option.label }}</span>
            <small v-if="option.description" class="question-option-description">{{ option.description }}</small>
          </span>
        </label>
        <input
          v-if="current.allowCustom"
          class="ui-field question-other"
          data-size="md"
          type="text"
          :value="typed[current.id] ?? ''"
          :placeholder="t('question.otherPlaceholder')"
          :aria-label="t('question.other')"
          @input="typeAnswer(current, ($event.target as HTMLInputElement).value)"
        >
      </fieldset>
      <ol v-else class="question-review">
        <li v-for="(question, index) in props.question.questions" :key="question.id">
          <strong>{{ question.question }}</strong>
          <p>{{ [...(selected[question.id] ?? []), ...(typedAnswer(question) ? [typedAnswer(question)] : [])].join(', ') || t('question.unanswered') }}</p>
          <button type="button" class="ui-button" data-size="sm" data-variant="ghost" :disabled="busy || sent" :aria-label="t('question.editAnswer', { question: question.question })" @click="go(index)">{{ t('question.edit') }}</button>
        </li>
      </ol>
      </div>
      <p v-if="error" class="prompt-error" role="alert">{{ error }}</p>
      <div class="prompt-buttons">
        <button type="button" class="ui-button question-decline" data-size="md" data-variant="ghost" :disabled="busy || sent" @click="send(true)">{{ t("question.decline") }}</button>
        <button v-if="multiple && step > 0" type="button" class="ui-button" data-size="md" data-variant="secondary" :disabled="busy || sent" @click="go(step - 1)">{{ t('question.previous') }}</button>
        <button v-if="multiple && current" type="submit" class="ui-button" data-size="md" data-variant="primary" :disabled="busy || sent || !answered(current)">{{ step === props.question.questions.length - 1 ? t('question.review') : t('question.next') }}</button>
        <button v-else type="submit" class="ui-button" data-size="md" data-variant="primary" :disabled="busy || sent || !complete">{{ t("question.submit") }}</button>
      </div>
    </form>
  </article>
</template>

<style scoped>
.question-card { display: flex; min-height: 0; flex-direction: column; max-height: 56dvh; }
.prompt-header, .question-steps, .prompt-buttons, .prompt-error { flex: none; }
.question-steps { display: flex; gap: var(--space-1); overflow-x: auto; margin-top: var(--space-3); }
.question-steps .ui-button { flex: none; }
.question-steps [aria-current="step"] { background: var(--accent); color: var(--foreground); }
.question-form { display: flex; flex-direction: column; min-height: 0; margin-top: var(--space-3); }
.question-body { min-height: 0; overflow-y: auto; overflow-wrap: anywhere; }
.question-body:focus-visible { outline: 2px solid var(--ring); outline-offset: -2px; }
.question-decline { margin-right: auto; }
.question-review { display: grid; gap: var(--space-3); margin: 0; padding-left: var(--space-5); font-size: var(--text-sm); }
.question-review strong { font-weight: var(--weight-medium); }
.question-review p { margin: var(--space-1) 0; color: var(--muted-foreground); white-space: pre-wrap; }
.question-item { min-width: 0; margin: 0; padding: 0; border: 0; }
.question-item legend { margin-bottom: var(--space-2); font-size: var(--text-sm); font-weight: var(--weight-medium); }
.question-header { margin-right: var(--space-2); color: var(--muted-foreground); }
.question-hint { margin-left: var(--space-2); color: var(--muted-foreground); font-weight: var(--weight-regular); }
.question-option { display: flex; align-items: flex-start; gap: var(--space-2); padding: var(--space-1) 0; font-size: var(--text-sm); cursor: pointer; }
.question-option-text { display: grid; min-width: 0; gap: 2px; }
.question-option-description { color: var(--muted-foreground); }
.question-other { margin-top: var(--space-2); width: 100%; }
</style>
