<script setup lang="ts">
withDefaults(defineProps<{
  /** Host-provided display row; the divider only uses it for identity. */
  row?: unknown
  text?: string
}>(), {
  row: undefined,
  text: 'Part of this response cannot be rendered safely in this bounded view.',
})
</script>

<template>
  <div class="timeline-window-truncation" role="status">
    <span class="timeline-window-truncation__line" />
    <span>{{ text }}</span>
    <span class="timeline-window-truncation__line" />
  </div>
</template>

<style scoped>
.timeline-window-truncation {
  display: grid;
  grid-template-columns: minmax(24px, 1fr) auto minmax(24px, 1fr);
  align-items: center;
  gap: 12px;
  padding: 12px 0;
  color: var(--muted-foreground, #71717a);
  font-size: 12px;
  line-height: 1.4;
}

.timeline-window-truncation__line {
  height: 1px;
  background: var(--border, #e4e4e7);
}
</style>
