<script setup lang="ts">
withDefaults(defineProps<{
  errorText: string
  label: string
  retryTitle: string
  canRetry: boolean
  pending?: boolean
}>(), {
  pending: false,
})

const emit = defineEmits<{
  retry: []
}>()
</script>

<template>
  <div class="timeline-retry-notice" data-testid="timeline-retry-notice">
    <slot name="icon" />
    <span class="timeline-retry-notice__message" role="status">{{ errorText }}</span>
    <button
      v-if="canRetry"
      type="button"
      class="timeline-retry-notice__pill timeline-retry-notice__button"
      :disabled="pending"
      :title="retryTitle"
      :aria-label="retryTitle"
      @click="emit('retry')"
    >{{ label }}</button>
    <span v-else class="timeline-retry-notice__pill" aria-live="off">{{ label }}</span>
  </div>
</template>

<style scoped>
.timeline-retry-notice {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-width: 0;
  margin-bottom: 0.75rem;
  border: 1px solid var(--border, #e4e4e7);
  border-radius: 0.75rem;
  background: var(--background, #fff);
  padding: 0.75rem 1rem;
  color: var(--foreground, #09090b);
  font-size: 1rem;
  line-height: 1.5;
}

.timeline-retry-notice__message {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}

.timeline-retry-notice__pill {
  flex-shrink: 0;
  border-radius: 9999px;
  background: var(--muted, #f4f4f5);
  padding: 0.25rem 0.625rem;
  font-size: 0.875rem;
  line-height: 1.25rem;
  font-variant-numeric: tabular-nums;
}

.timeline-retry-notice__button {
  border: 0;
  color: inherit;
  cursor: pointer;
}

.timeline-retry-notice__button:hover {
  background: var(--accent, #f4f4f5);
}

.timeline-retry-notice__button:focus-visible {
  outline: 2px solid var(--ring, #a1a1aa);
  outline-offset: 2px;
}

.timeline-retry-notice__button:disabled {
  cursor: default;
  opacity: 0.5;
}
</style>
