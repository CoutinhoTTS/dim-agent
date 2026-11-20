<script setup lang="ts">
/**
 * Height-animated region for one collapsible work-details row of a turn.
 *
 * The turn's work details are not a single region: the rows that disappear
 * when the turn collapses (work groups and the intermediate text blocks) are
 * interleaved with rows that stay visible (final text, generated image, plan
 * draft, notification, MCP tool). Wrapping the whole blocks container would
 * clip the always-visible rows, so each collapsible row owns its own grid
 * region while `collapsible=false` renders the slot without a wrapper — DOM
 * order and residual layout stay exactly as they are.
 */
defineProps<{
  collapsible: boolean
  open: boolean
}>()
</script>

<template>
  <div
    v-if="collapsible"
    class="collapse-grid"
    :class="{ 'collapse-grid-open': open }"
    data-testid="timeline-details-region"
  >
    <div class="collapse-grid-inner">
      <slot />
    </div>
  </div>
  <slot v-else />
</template>

<style scoped>
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
</style>
