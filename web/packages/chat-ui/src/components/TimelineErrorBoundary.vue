<script setup lang="ts">
import { onErrorCaptured, ref } from 'vue'

const props = defineProps<{
  fallbackText: string
}>()

const renderError = ref<unknown>(null)

/**
 * Per-row boundary for the timeline render tree. The fallback is sticky until
 * the row is unmounted or re-keyed; a single bad row must not take down the
 * whole timeline.
 *
 * Accepted trade-off: `onErrorCaptured` sees every descendant error — render,
 * lifecycle, event-handler and watcher alike — so a one-off handler error also
 * pins the fallback row. Row handlers are host commands whose throw would
 * otherwise surface only as a console entry; trading that row for a visible
 * error state is the intended blast-radius control.
 */
onErrorCaptured((error) => {
  renderError.value = error
  console.error('[chat-ui][timeline] row render failed', error)
  return false
})
</script>

<template>
  <slot v-if="renderError === null" />
  <div
    v-else
    class="timeline-row-error"
    data-testid="timeline-row-render-error"
    role="alert"
  >
    <slot name="icon" />
    <span>{{ fallbackText }}</span>
  </div>
</template>

<style scoped>
.timeline-row-error {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  border: 1px solid var(--border, #e4e4e7);
  border-radius: 0.5rem;
  background: color-mix(in srgb, var(--muted, #f4f4f5) 40%, transparent);
  padding: 0.5rem 0.75rem;
  color: var(--muted-foreground, #71717a);
  font-size: 0.875rem;
  line-height: 1.25rem;
}
</style>
