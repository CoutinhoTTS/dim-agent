import { onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'
import { latestThinkingFragment } from './thinking-excerpt'

const EXCERPT_THROTTLE_MS = 500

export interface ThinkingExcerptState {
  text: Ref<string>
  /** The source text was longer than the displayed fragment. */
  truncated: Ref<boolean>
  /** Apply the current content immediately (e.g. when the source block changes). */
  flush: () => void
}

/**
 * Bounded tail excerpt for a collapsed thinking row or work-group header.
 *
 * While `active` is true, content deltas coalesce into at most one write per
 * 500 ms window; when it turns false the final fragment flushes immediately.
 * Writes are skipped while the window is hidden and flushed when it becomes
 * visible again. The full reasoning stays behind the caller's expand UI.
 */
export function useThinkingExcerpt(
  content: () => string,
  active: () => boolean = () => true,
): ThinkingExcerptState {
  const text = ref('')
  const truncated = ref(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  let lastAppliedAt = 0
  let initialized = false

  function apply(): void {
    if (document.visibilityState !== 'visible') return
    const fragment = latestThinkingFragment(content())
    text.value = fragment.text
    truncated.value = fragment.truncated
    lastAppliedAt = Date.now()
  }

  function flush(): void {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
    apply()
  }

  function schedule(): void {
    if (timer !== undefined) return
    // Nothing to render while backgrounded; visibilitychange flushes the latest text.
    if (document.visibilityState !== 'visible') return
    const wait = Math.max(0, EXCERPT_THROTTLE_MS - (Date.now() - lastAppliedAt))
    timer = setTimeout(() => {
      timer = undefined
      apply()
    }, wait)
  }

  watch(content, () => {
    if (!initialized) {
      initialized = true
      flush()
      return
    }
    if (active()) schedule()
    else flush()
  }, { immediate: true })
  watch(active, isActive => {
    if (isActive) schedule()
    else flush()
  })

  function handleVisibilityChange(): void {
    if (document.visibilityState === 'visible') flush()
  }

  onMounted(() => document.addEventListener('visibilitychange', handleVisibilityChange))
  onBeforeUnmount(() => {
    document.removeEventListener('visibilitychange', handleVisibilityChange)
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
  })

  return { text, truncated, flush }
}
