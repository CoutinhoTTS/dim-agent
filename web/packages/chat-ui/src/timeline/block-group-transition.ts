import { computed, onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'

export const DEFAULT_TIMELINE_BLOCK_GROUP_ANIMATION_MS = 300
export const DEFAULT_TIMELINE_BLOCK_GROUP_LAZY_THRESHOLD = 32

export interface TimelineBlockGroupTransitionOptions {
  expanded: MaybeRefOrGetter<boolean>
  blockCount: MaybeRefOrGetter<number>
  groupAnimationMs?: number
  lazyBlockThreshold?: number
  scheduleFrame?: (callback: FrameRequestCallback) => number
  cancelFrame?: (handle: number) => void
}

function defaultScheduleFrame(callback: FrameRequestCallback): number {
  if (typeof requestAnimationFrame === 'function') {
    return requestAnimationFrame(callback)
  }
  return setTimeout(() => callback(Date.now()), 16) as unknown as number
}

function defaultCancelFrame(handle: number): void {
  if (typeof cancelAnimationFrame === 'function') {
    cancelAnimationFrame(handle)
  } else {
    clearTimeout(handle)
  }
}

/**
 * Lazy mount/unmount state for a large block group's collapsed content.
 *
 * A collapsed group is only a summary row. Content above the lazy threshold
 * stays mounted for the close animation and is then unmounted; a group opened
 * while unmounted mounts first with `contentOpen = false`, then opens over two
 * animation frames so the collapse-grid transition can run. Small groups stay
 * mounted because their DOM is negligible and remounting them adds churn.
 */
export function useTimelineBlockGroupTransition(options: TimelineBlockGroupTransitionOptions) {
  const groupAnimationMs = options.groupAnimationMs ?? DEFAULT_TIMELINE_BLOCK_GROUP_ANIMATION_MS
  const lazyBlockThreshold = options.lazyBlockThreshold ?? DEFAULT_TIMELINE_BLOCK_GROUP_LAZY_THRESHOLD
  const scheduleFrame = options.scheduleFrame ?? defaultScheduleFrame
  const cancelFrame = options.cancelFrame ?? defaultCancelFrame

  const lazyUnmountContent = computed(() => toValue(options.blockCount) >= lazyBlockThreshold)
  const contentMounted = ref(toValue(options.expanded) || !lazyUnmountContent.value)
  const contentOpen = ref(toValue(options.expanded))

  let collapseTimer: ReturnType<typeof setTimeout> | undefined
  let openFrame: number | undefined

  function cancelTransition(): void {
    if (collapseTimer !== undefined) {
      clearTimeout(collapseTimer)
      collapseTimer = undefined
    }
    if (openFrame !== undefined) {
      cancelFrame(openFrame)
      openFrame = undefined
    }
  }

  watch(() => [toValue(options.expanded), lazyUnmountContent.value] as const, ([expanded, lazy]) => {
    cancelTransition()
    if (!lazy) {
      contentMounted.value = true
      contentOpen.value = expanded
      return
    }
    if (expanded) {
      if (contentMounted.value) {
        contentOpen.value = true
        return
      }
      contentMounted.value = true
      contentOpen.value = false
      openFrame = scheduleFrame(() => {
        openFrame = scheduleFrame(() => {
          openFrame = undefined
          contentOpen.value = true
        })
      })
      return
    }
    contentOpen.value = false
    if (!contentMounted.value) return
    collapseTimer = setTimeout(() => {
      collapseTimer = undefined
      contentMounted.value = false
    }, groupAnimationMs)
  })

  onScopeDispose(cancelTransition)

  return {
    contentMounted,
    contentOpen,
    lazyUnmountContent,
  }
}
