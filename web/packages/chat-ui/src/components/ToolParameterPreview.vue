<script setup lang="ts">
defineProps<{
  lines: string[]
}>()
</script>

<template>
  <div
    v-if="lines.length > 0"
    class="tool-parameter-preview"
    data-testid="tool-parameter-preview"
  >
    <div
      v-for="(line, index) in lines"
      :key="index"
      class="tool-parameter-preview__line"
    >
      {{ line }}
    </div>
  </div>
</template>

<style scoped>
.tool-parameter-preview {
  max-height: 5rem;
  overflow: hidden;
  color: var(--muted-foreground, #71717a);
  font-size: 0.75rem;
  line-height: 1.25rem;
}

.tool-parameter-preview__line {
  white-space: pre-wrap;
  overflow-wrap: break-word;
}
</style>
