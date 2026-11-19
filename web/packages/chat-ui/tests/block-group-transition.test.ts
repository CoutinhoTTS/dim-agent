import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { useTimelineBlockGroupTransition } from '../src'

function createFrameQueue() {
  let nextHandle = 1
  const callbacks = new Map<number, FrameRequestCallback>()
  return {
    callbacks,
    scheduleFrame: vi.fn((callback: FrameRequestCallback) => {
      const handle = nextHandle
      nextHandle += 1
      callbacks.set(handle, callback)
      return handle
    }),
    cancelFrame: vi.fn((handle: number) => {
      callbacks.delete(handle)
    }),
    flushFrame() {
      const pending = [...callbacks.entries()]
      callbacks.clear()
      for (const [, callback] of pending) callback(0)
    },
  }
}

afterEach(() => {
  vi.useRealTimers()
})

describe('useTimelineBlockGroupTransition', () => {
  it('keeps small expanded groups mounted without scheduling frames', async () => {
    const frames = createFrameQueue()
    const expanded = ref(true)
    const blockCount = ref(5)
    const scope = effectScope()
    const state = scope.run(() => useTimelineBlockGroupTransition({
      expanded,
      blockCount,
      scheduleFrame: frames.scheduleFrame,
      cancelFrame: frames.cancelFrame,
    }))!

    expect(state.lazyUnmountContent.value).toBe(false)
    expect(state.contentMounted.value).toBe(true)
    expect(state.contentOpen.value).toBe(true)

    expanded.value = false
    await nextTick()
    expect(state.contentMounted.value).toBe(true)
    expect(state.contentOpen.value).toBe(false)
    expect(frames.scheduleFrame).not.toHaveBeenCalled()
    scope.stop()
  })

  it('delays unmounting a large collapsed group by the close animation', async () => {
    vi.useFakeTimers()
    const frames = createFrameQueue()
    const expanded = ref(false)
    const blockCount = ref(40)
    const scope = effectScope()
    const state = scope.run(() => useTimelineBlockGroupTransition({
      expanded,
      blockCount,
      scheduleFrame: frames.scheduleFrame,
      cancelFrame: frames.cancelFrame,
    }))!

    expect(state.lazyUnmountContent.value).toBe(true)
    expect(state.contentMounted.value).toBe(false)
    expect(state.contentOpen.value).toBe(false)

    expanded.value = true
    await nextTick()
    expect(state.contentMounted.value).toBe(true)
    expect(state.contentOpen.value).toBe(false)

    frames.flushFrame()
    expect(state.contentOpen.value).toBe(false)
    frames.flushFrame()
    expect(state.contentOpen.value).toBe(true)

    expanded.value = false
    await nextTick()
    expect(state.contentOpen.value).toBe(false)
    expect(state.contentMounted.value).toBe(true)

    vi.advanceTimersByTime(299)
    expect(state.contentMounted.value).toBe(true)
    vi.advanceTimersByTime(1)
    expect(state.contentMounted.value).toBe(false)
    scope.stop()
  })

  it('cancels pending frames and unmount timers when the scope stops', async () => {
    vi.useFakeTimers()
    const frames = createFrameQueue()
    const expanded = ref(false)
    const scope = effectScope()
    scope.run(() => useTimelineBlockGroupTransition({
      expanded,
      blockCount: ref(40),
      scheduleFrame: frames.scheduleFrame,
      cancelFrame: frames.cancelFrame,
    }))

    expanded.value = true
    await nextTick()
    expect(frames.callbacks.size).toBe(1)
    scope.stop()
    expect(frames.cancelFrame).toHaveBeenCalledTimes(1)
    expect(frames.callbacks.size).toBe(0)

    const collapseExpanded = ref(true)
    const collapseScope = effectScope()
    const collapseState = collapseScope.run(() => useTimelineBlockGroupTransition({
      expanded: collapseExpanded,
      blockCount: ref(40),
      scheduleFrame: frames.scheduleFrame,
      cancelFrame: frames.cancelFrame,
    }))!

    collapseExpanded.value = false
    await nextTick()
    expect(collapseState.contentMounted.value).toBe(true)
    collapseScope.stop()
    vi.advanceTimersByTime(300)
    expect(collapseState.contentMounted.value).toBe(true)
  })
})
