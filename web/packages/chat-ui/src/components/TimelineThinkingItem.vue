<script setup lang="ts">
import { computed, ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  blockId: string
  content: string
  isRunning: boolean
  expanded: boolean
  titleText: string
  excerpt: string
  excerptTruncated?: boolean
}>(), {
  excerptTruncated: false,
})

const emit = defineEmits<{
  toggle: []
}>()

// Keep-mounted latch (interaction spec §9.1): once expanded, the quote stays
// mounted so the collapse grid animates over real content instead of
// unmounting in the same frame.
const quoteMounted = ref(props.expanded)
watch(() => props.expanded, (value) => {
  if (value) quoteMounted.value = true
})

// Expanded + streaming: keep the quote pinned to the newest reasoning. The
// scroll animates (smooth) so the text visibly flows instead of teleporting;
// rAF throttle keeps one scroll command per frame under delta bursts. User
// scrolling up pauses the pin until they return to the bottom.
const quoteRef = ref<HTMLElement | null>(null)
let followScrollScheduled = false

function isPinnedToBottom(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= 32
}

const terminalContent = computed(() => props.content.trim())

watch([terminalContent, () => props.expanded], ([, expandedValue]) => {
  if (!expandedValue || !props.isRunning) return
  const element = quoteRef.value
  if (!element || !isPinnedToBottom(element)) return
  if (followScrollScheduled) return
  followScrollScheduled = true
  requestAnimationFrame(() => {
    followScrollScheduled = false
    if (!quoteRef.value) return
    quoteRef.value.scrollTo({ top: quoteRef.value.scrollHeight, behavior: 'smooth' })
  })
}, { flush: 'post' })

// Freshly expanded during a run: jump to the bottom without animation so the
// user starts reading from the newest reasoning, not the top.
watch(() => props.expanded, (value) => {
  if (!value || !props.isRunning) return
  requestAnimationFrame(() => {
    const element = quoteRef.value
    if (!element || !isPinnedToBottom(element)) return
    element.scrollTop = element.scrollHeight
  })
})
</script>

<template>
  <div class="timeline-thinking-block" data-testid="timeline-thinking-block" :data-block-id="blockId">
    <button
      type="button"
      class="thinking-toggle"
      :aria-label="titleText"
      :aria-expanded="expanded"
      @click="emit('toggle')"
    >
      <slot name="icon" />
      <span class="thinking-title" :class="{ 'shimmer-text': isRunning }">
        {{ titleText }}
      </span>
      <span
        v-if="!expanded && excerpt"
        class="thinking-separator"
        aria-hidden="true"
        data-testid="timeline-thinking-separator"
      >·</span>
      <span
        v-if="!expanded"
        class="thinking-excerpt"
        :class="{ 'has-source-gate': excerptTruncated && excerpt }"
        :title="excerpt"
        data-testid="timeline-thinking-excerpt"
      >
        <span class="thinking-excerpt-text" data-testid="timeline-thinking-excerpt-text">
          <bdi dir="ltr">{{ excerpt }}</bdi>
        </span>
      </span>
      <slot name="chevron" />
    </button>
    <div
      v-if="quoteMounted"
      class="collapse-grid"
      :class="{ 'collapse-grid-open': expanded }"
    >
      <div class="collapse-grid-inner">
        <div class="thinking-quote-wrap">
          <div
            ref="quoteRef"
            class="thinking-quote"
            data-testid="timeline-thinking-quote"
          >{{ terminalContent }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline-thinking-block {
  margin-bottom: 0.75rem;
}

.thinking-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: flex-start;
  /* Match ToolCallInlineSummary: 4px gap + the icon's 2px right margin. */
  gap: 0.25rem;
  width: 100%;
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--muted-foreground, #71717a);
  font: inherit;
  font-size: 1rem;
  font-weight: 400;
  line-height: 1.5;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  outline: none;
  transition: color 150ms;
}

.thinking-toggle:hover {
  background: transparent;
  color: var(--foreground, #09090b);
}

.thinking-toggle:focus-visible {
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring, #a1a1aa) 50%, transparent);
}

.thinking-toggle :deep(svg) {
  pointer-events: none;
  flex-shrink: 0;
}

.thinking-title {
  display: inline-flex;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}

.shimmer-text {
  background: linear-gradient(
    90deg,
    currentColor 0%,
    currentColor 40%,
    var(--activity-shimmer-highlight) 50%,
    currentColor 60%,
    currentColor 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer 2s steps(48, end) infinite;
}

@keyframes shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}

.collapse-grid {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 300ms cubic-bezier(0.16, 1, 0.3, 1);
}

.collapse-grid-open {
  grid-template-rows: 1fr;
}

.collapse-grid-inner {
  overflow: hidden;
  min-height: 0;
  opacity: 0;
  transition: opacity 300ms ease-in;
}

.collapse-grid-open .collapse-grid-inner {
  opacity: 1;
}

.thinking-quote-wrap {
  padding: 0.25rem 0;
}

.thinking-quote {
  max-height: 240px;
  overflow: auto;
  border-left: 2px solid color-mix(in srgb, var(--border, #e4e4e7) 70%, transparent);
  padding-left: 0.75rem;
  color: var(--muted-foreground, #71717a);
  font-size: 1rem;
  line-height: 1.625;
  white-space: pre-wrap;
  overflow-wrap: break-word;
}

.thinking-separator {
  flex: none;
  color: var(--muted-foreground, #71717a);
  opacity: 0.55;
}

.thinking-excerpt {
  display: flex;
  align-items: center;
  gap: 3px;
  min-width: 0;
  flex: 1;
  overflow: hidden;
  font-size: 0.875rem;
  line-height: 1.35;
  color: var(--muted-foreground, #71717a);
}

.thinking-excerpt.has-source-gate::before {
  content: "…";
  flex: none;
  color: var(--muted-foreground, #71717a);
  font-family: var(--font-mono);
  font-size: 10px;
  pointer-events: none;
}

.thinking-excerpt-text {
  display: block;
  min-width: 0;
  flex: 1;
  overflow: hidden;
  white-space: nowrap;
  text-align: left;
  direction: rtl;
  text-overflow: ellipsis;
}

.thinking-excerpt.has-source-gate .thinking-excerpt-text {
  text-overflow: clip;
}
</style>
