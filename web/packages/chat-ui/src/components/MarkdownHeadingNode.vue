<script setup lang="ts">
import { computed } from 'vue'
import { HeadingNode } from 'markstream-vue'
import { githubHeadingSlug } from '../utils/markdown-heading-slug'
import type { MarkdownHeadingAst } from './markdown-heading-node'

const props = defineProps<{
  node: MarkdownHeadingAst
  customId?: string
  indexKey?: string | number
}>()

const resolvedNode = computed<MarkdownHeadingAst>(() => {
  const existingId = props.node.attrs?.id
  if (existingId) {
    return props.node
  }
  const slug = githubHeadingSlug(props.node.text)
  if (!slug) {
    return props.node
  }
  return {
    ...props.node,
    attrs: { ...props.node.attrs, id: slug },
  }
})
</script>

<template>
  <HeadingNode :node="resolvedNode" :custom-id="customId" :index-key="indexKey" />
</template>
