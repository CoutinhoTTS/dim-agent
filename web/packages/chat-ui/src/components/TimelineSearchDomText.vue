<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import {
  normalizeTimelineSearchQuery,
  type ChatUiTimelineSearchMatch,
} from '../timeline/search'

const props = defineProps<{
  query: string
  matches: readonly ChatUiTimelineSearchMatch[]
  activeMatchId: string | null
}>()

const rootRef = ref<HTMLElement | null>(null)

watch(
  () => [props.query, props.matches, props.activeMatchId] as const,
  () => {
    void nextTick(applyHighlights)
  },
  { deep: true, immediate: true, flush: 'post' },
)

// The markdown body can render asynchronously, so re-apply highlights when
// rendered text nodes appear. Batched per animation frame to avoid observer
// loops.
let highlightMutationObserver: MutationObserver | null = null
let highlightReapplyScheduled = false
function scheduleHighlightReapply(): void {
  if (highlightReapplyScheduled) {
    return
  }
  highlightReapplyScheduled = true
  requestAnimationFrame(() => {
    highlightReapplyScheduled = false
    applyHighlights()
  })
}

watch(
  rootRef,
  (root) => {
    highlightMutationObserver?.disconnect()
    highlightMutationObserver = null
    if (root && normalizeTimelineSearchQuery(props.query)) {
      attachHighlightObserver(root)
    }
  },
  { immediate: true },
)

watch(
  () => props.query,
  () => {
    const root = rootRef.value
    if (!root) {
      return
    }
    if (normalizeTimelineSearchQuery(props.query)) {
      attachHighlightObserver(root)
    } else {
      highlightMutationObserver?.disconnect()
      highlightMutationObserver = null
    }
  },
)

onBeforeUnmount(() => {
  highlightMutationObserver?.disconnect()
  highlightMutationObserver = null
  if (rootRef.value) {
    clearHighlights(rootRef.value)
  }
})

function applyHighlights() {
  const root = rootRef.value
  if (!root) {
    return
  }
  highlightMutationObserver?.disconnect()

  try {
    const query = normalizeTimelineSearchQuery(props.query)
    if (!query) {
      const existing = root.querySelector<HTMLElement>('[data-timeline-search-generated="true"]')
      if (existing) {
        clearHighlights(root)
      }
      return
    }

    clearHighlights(root)

    const queryLower = query.toLocaleLowerCase()
    const textNodes = collectTextNodes(root)
    let occurrenceIndex = 0

    for (const node of textNodes) {
      const text = node.nodeValue ?? ''
      const textLower = text.toLocaleLowerCase()
      const ranges: Array<{
        start: number
        end: number
        match?: ChatUiTimelineSearchMatch
      }> = []

      let searchFrom = 0
      while (searchFrom <= textLower.length) {
        const start = textLower.indexOf(queryLower, searchFrom)
        if (start === -1) {
          break
        }

        const match = props.matches.find(
          candidate => candidate.sourceMatchIndex === occurrenceIndex,
        )
        ranges.push({ start, end: start + query.length, match })
        occurrenceIndex += 1
        searchFrom = start + query.length
      }

      for (const range of ranges.reverse()) {
        wrapTextRange(node, range.start, range.end, range.match)
      }
    }
  } finally {
    const rootElement = rootRef.value
    if (rootElement) {
      attachHighlightObserver(rootElement)
    }
  }
}

function attachHighlightObserver(root: HTMLElement): void {
  highlightMutationObserver?.disconnect()
  if (typeof MutationObserver !== 'undefined') {
    highlightMutationObserver = new MutationObserver(scheduleHighlightReapply)
    highlightMutationObserver.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
    })
  }
}

function clearHighlights(root: HTMLElement) {
  const highlights = Array.from(
    root.querySelectorAll<HTMLElement>('[data-timeline-search-generated="true"]'),
  )

  for (const highlight of highlights) {
    const parent = highlight.parentNode
    if (!parent) {
      continue
    }

    while (highlight.firstChild) {
      parent.insertBefore(highlight.firstChild, highlight)
    }
    parent.removeChild(highlight)
    parent.normalize()
  }
}

function collectTextNodes(root: HTMLElement): Text[] {
  const nodes: Text[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement
      if (!parent || shouldSkipElement(parent)) {
        return NodeFilter.FILTER_REJECT
      }
      return node.nodeValue ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT
    },
  })

  let node = walker.nextNode()
  while (node) {
    nodes.push(node as Text)
    node = walker.nextNode()
  }

  return nodes
}

function shouldSkipElement(element: Element): boolean {
  return Boolean(
    element.closest(
      '[data-timeline-search-generated="true"], button, input, textarea, select, [contenteditable="true"]',
    ),
  )
}

function wrapTextRange(
  node: Text,
  startOffset: number,
  endOffset: number,
  match?: ChatUiTimelineSearchMatch,
) {
  const range = document.createRange()
  range.setStart(node, startOffset)
  range.setEnd(node, endOffset)

  const highlight = document.createElement('span')
  highlight.dataset.timelineSearchGenerated = 'true'
  if (match) {
    highlight.dataset.timelineSearchMatchId = match.id
  }
  highlight.className =
    match?.id === props.activeMatchId
      ? 'timeline-search-match timeline-search-match-active'
      : 'timeline-search-match'

  range.surroundContents(highlight)
}
</script>

<template>
  <div ref="rootRef" class="contents">
    <slot />
  </div>
</template>
