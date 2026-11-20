<script setup lang="ts">
defineOptions({
  name: 'TimelineSkeleton',
})
</script>

<template>
  <div class="timeline-skeleton" data-testid="timeline-skeleton">
    <slot name="brand" />
  </div>
</template>

<style scoped>
.timeline-skeleton {
  display: flex;
  height: 100%;
  width: 100%;
  align-items: center;
  justify-content: center;
}
</style>
