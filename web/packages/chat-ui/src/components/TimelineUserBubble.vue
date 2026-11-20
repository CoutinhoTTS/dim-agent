<script setup lang="ts">
defineProps<{ messageId: string }>()
</script>

<template>
  <div
    class="timeline-user-bubble"
    :data-timeline-user-message-visible-id="messageId"
  ><slot /></div>
</template>

<!-- Keep this class global: desktop skins override `.skin-active .timeline-user-bubble`. -->
<style>
.timeline-user-bubble {
  min-width: 0;
  max-width: 100%;
  border-radius: 0.5rem;
  background-color: var(--muted, #f4f4f5);
  padding: 0.5rem 1rem;
  color: var(--foreground, inherit);
  font-size: 1rem;
  line-height: 1.625;
}
</style>
