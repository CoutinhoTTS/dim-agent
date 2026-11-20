<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type {
  ChatUiRunChangedPathEntry,
  ChatUiTimelineUserMessageMarker as TimelineUserMessageMarker,
} from '../timeline/user-message-rail'
import { toAssistantPreview } from '../timeline/user-message-rail'
import type { ChatUiTimelineUserMessageRailProps } from './user-message-rail'

type ChangedPathsState =
  | { status: 'loading' }
  | { status: 'loaded'; paths: ChatUiRunChangedPathEntry[] }
  | { status: 'unavailable' }

const props = withDefaults(defineProps<ChatUiTimelineUserMessageRailProps>(), {
  scrollParent: null,
  timelineRoot: null,
  rowDomSignature: '',
  layoutResizing: false,
  readMessageDetail: undefined,
  readRunChangedPaths: undefined,
})

const emit = defineEmits<{
  'marker-click': [marker: TimelineUserMessageMarker]
}>()

const changedPathsCache = new Map<string, ChangedPathsState>()
const messagePreviewCache = new Map<string, { markerId: string; preview: string; assistantPreview?: string }>()
const messagePreviewLoadingKeys = new Set<string>()
const activeMarkerId = ref<string | null>(null)
const nearMarkerIds = ref<Set<string>>(new Set())
const changedPathsByMarkerId = ref<Record<string, ChangedPathsState>>({})
const messagePreviewByMarkerId = ref<Record<string, string>>({})
const assistantPreviewByMarkerId = ref<Record<string, string>>({})
let suppressedMarkerHoverId: string | null = null
let densePreviewTimer: ReturnType<typeof setTimeout> | null = null
const denseCanvasRef = ref<HTMLCanvasElement | null>(null)
const denseActiveIndex = ref(0)
const RAIL_MAX_HEIGHT_PX = 420
const RAIL_PADDING_PX = 5
const RAIL_MARKER_HEIGHT_PX = 2
const RAIL_MARKER_GAP_PX = 2
const RAIL_SLOT_PITCH_PX = RAIL_MARKER_HEIGHT_PX + RAIL_MARKER_GAP_PX
const RAIL_MARKER_WIDTHS_PX = [26, 20, 14, 10] as const
const RAIL_MARKER_DEFAULT_WIDTH_PX = 6
const RAIL_WIDTH_PX = RAIL_MARKER_WIDTHS_PX[0]
const DENSE_RAIL_SLOT_COUNT = Math.floor(
  (RAIL_MAX_HEIGHT_PX - RAIL_PADDING_PX * 2 + RAIL_MARKER_GAP_PX) / RAIL_SLOT_PITCH_PX,
)

const loadedMarkerById = computed(() => new Map(
  props.loadedMarkers.map(marker => [marker.userMessageId, marker]),
))
const markerStructureSignature = computed(() => JSON.stringify(
  props.markers.map(marker => [marker.id, marker.userMessageId]),
))
const loadedMarkerMappingSignature = computed(() => JSON.stringify(
  props.loadedMarkers.map(marker => [marker.id, marker.visibleRowKeys]),
))
const markerOrdinalById = new Map<string, number>()
const markerIds = new Set<string>()

function resolveMarker(marker: TimelineUserMessageMarker): TimelineUserMessageMarker {
  return loadedMarkerById.value.get(marker.userMessageId) ?? marker
}

const denseRail = computed(() => props.markers.length > DENSE_RAIL_SLOT_COUNT)
const displayMarkers = computed(() => denseRail.value
  ? props.markers
  : props.markers.map(resolveMarker))
const denseActiveMarker = computed(() => {
  const marker = props.markers[denseActiveIndex.value]
  return marker ? resolveMarker(marker) : null
})
const denseActiveTop = computed(() => markerTopPx(denseActiveIndex.value))
let timelineIntersectionObserver: IntersectionObserver | null = null
const intersectingRowKeys = new Set<string>()
const markerIdsByRowKey = new Map<string, string[]>()
const nearMarkerRefCount = new Map<string, number>()

const sparseMarkerPitchPx = computed(() => {
  const markerCount = props.markers.length
  const preferredPitchPx = markerCount > 80
    ? 4
    : markerCount > 60
      ? 5
      : markerCount > 40
        ? 8
        : markerCount > 24
          ? 10
          : 12
  if (markerCount <= 1) return preferredPitchPx
  const maximumPitchPx = (
    RAIL_MAX_HEIGHT_PX - RAIL_PADDING_PX * 2 - RAIL_MARKER_HEIGHT_PX
  ) / (markerCount - 1)
  return Math.min(preferredPitchPx, maximumPitchPx)
})

const railMarkerPitchPx = computed(() => (
  denseRail.value ? RAIL_SLOT_PITCH_PX : sparseMarkerPitchPx.value
))

const railHeightPx = computed(() => {
  const markerCount = props.markers.length
  if (markerCount <= 0) return RAIL_PADDING_PX * 2
  return RAIL_PADDING_PX * 2
    + RAIL_MARKER_HEIGHT_PX
    + (Math.min(markerCount, DENSE_RAIL_SLOT_COUNT) - 1) * railMarkerPitchPx.value
})

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path
}

function cacheKey(sessionId: string, runId: string): string {
  return `${sessionId}:${runId}`
}

function markerPreview(marker: TimelineUserMessageMarker): string {
  return messagePreviewByMarkerId.value[marker.id] ?? marker.preview
}

function markerAssistantPreview(marker: TimelineUserMessageMarker): string | undefined {
  return assistantPreviewByMarkerId.value[marker.id] ?? marker.assistantPreview
}

async function loadMessagePreview(marker: TimelineUserMessageMarker): Promise<void> {
  const sessionId = props.sessionId
  const readMessageDetail = props.readMessageDetail
  if (!sessionId || marker.previewLoaded !== false || readMessageDetail === undefined) return
  const key = `${sessionId}:${marker.userMessageId}`
  const cached = messagePreviewCache.get(key)
  if (cached !== undefined) {
    messagePreviewByMarkerId.value = { ...messagePreviewByMarkerId.value, [marker.id]: cached.preview }
    if (cached.assistantPreview !== undefined) {
      assistantPreviewByMarkerId.value = { ...assistantPreviewByMarkerId.value, [marker.id]: cached.assistantPreview }
    }
    return
  }
  if (messagePreviewLoadingKeys.has(key)) return
  messagePreviewLoadingKeys.add(key)
  try {
    const result = await readMessageDetail({
      sessionId,
      messageId: marker.userMessageId,
    })
    if (result === null || result === undefined) return
    if (props.sessionId !== sessionId || !markerIds.has(marker.id)) return
    const preview = result.message.content.replace(/\s+/g, ' ').trim().slice(0, 180)
    if (messagePreviewCache.size >= 200) {
      const oldestKey = messagePreviewCache.keys().next().value
      if (oldestKey !== undefined) {
        const oldest = messagePreviewCache.get(oldestKey)
        messagePreviewCache.delete(oldestKey)
        if (oldest && messagePreviewByMarkerId.value[oldest.markerId] === oldest.preview) {
          const next = { ...messagePreviewByMarkerId.value }
          delete next[oldest.markerId]
          messagePreviewByMarkerId.value = next
        }
        if (oldest?.assistantPreview !== undefined && assistantPreviewByMarkerId.value[oldest.markerId] === oldest.assistantPreview) {
          const next = { ...assistantPreviewByMarkerId.value }
          delete next[oldest.markerId]
          assistantPreviewByMarkerId.value = next
        }
      }
    }
    const processedAssistantPreview = result.assistantPreview !== undefined
      ? toAssistantPreview(result.assistantPreview)
      : undefined
    messagePreviewCache.set(key, { markerId: marker.id, preview, assistantPreview: processedAssistantPreview })
    messagePreviewByMarkerId.value = { ...messagePreviewByMarkerId.value, [marker.id]: preview }
    if (processedAssistantPreview !== undefined) {
      assistantPreviewByMarkerId.value = { ...assistantPreviewByMarkerId.value, [marker.id]: processedAssistantPreview }
    }
  } catch {
    // Keep the ordinal placeholder when the target changed while details loaded.
  } finally {
    messagePreviewLoadingKeys.delete(key)
  }
}

function markerTopPx(index: number): string {
  const count = props.markers.length
  const height = railHeightPx.value
  if (count <= 1) {
    return `${height / 2}px`
  }
  const firstMarkerCenterPx = RAIL_PADDING_PX + RAIL_MARKER_HEIGHT_PX / 2
  if (denseRail.value) {
    const slotIndex = denseSlotIndex(index)
    return `${firstMarkerCenterPx + slotIndex * RAIL_SLOT_PITCH_PX}px`
  }
  return `${firstMarkerCenterPx + index * railMarkerPitchPx.value}px`
}

function markerWidthPx(index: number): number {
  const activeIndex = activeMarkerId.value === null
    ? undefined
    : markerOrdinalById.get(activeMarkerId.value)
  if (activeIndex === undefined) return RAIL_MARKER_DEFAULT_WIDTH_PX
  return RAIL_MARKER_WIDTHS_PX[Math.abs(activeIndex - index)] ?? RAIL_MARKER_DEFAULT_WIDTH_PX
}

function denseMarkerIndex(clientY: number): number {
  const canvas = denseCanvasRef.value
  if (!canvas || props.markers.length <= 1) return 0
  const rect = canvas.getBoundingClientRect()
  const firstMarkerCenterPx = RAIL_PADDING_PX + RAIL_MARKER_HEIGHT_PX / 2
  const ratio = Math.min(
    1,
    Math.max(
      0,
      (clientY - rect.top - firstMarkerCenterPx) / Math.max(1, rect.height - firstMarkerCenterPx * 2),
    ),
  )
  return Math.round(ratio * (props.markers.length - 1))
}

function denseSlotIndex(markerIndex: number): number {
  if (props.markers.length <= 1) return 0
  return Math.round(
    (markerIndex * (DENSE_RAIL_SLOT_COUNT - 1)) / (props.markers.length - 1),
  )
}

function denseSlotTopPx(slotIndex: number): number {
  return RAIL_PADDING_PX + slotIndex * RAIL_SLOT_PITCH_PX
}

function drawDenseRail() {
  const canvas = denseCanvasRef.value
  if (!canvas || !denseRail.value) return
  const width = RAIL_WIDTH_PX
  const height = railHeightPx.value
  const scale = window.devicePixelRatio || 1
  canvas.width = Math.round(width * scale)
  canvas.height = Math.round(height * scale)
  canvas.style.width = `${width}px`
  canvas.style.height = `${height}px`
  const context = canvas.getContext('2d')
  if (!context) return
  context.setTransform(scale, 0, 0, scale, 0, 0)
  context.clearRect(0, 0, width, height)
  const activeMarkerIndex = activeMarkerId.value === null
    ? undefined
    : markerOrdinalById.get(activeMarkerId.value)
  const activeSlotIndex = activeMarkerIndex === undefined
    ? undefined
    : denseSlotIndex(activeMarkerIndex)
  const emphasizedSlotIndexes = new Set<number>()
  for (const marker of props.loadedMarkers) {
    if (!nearMarkerIds.value.has(marker.id)) continue
    const markerIndex = markerOrdinalById.get(marker.userMessageId)
    if (markerIndex !== undefined) emphasizedSlotIndexes.add(denseSlotIndex(markerIndex))
  }
  for (let slotIndex = 0; slotIndex < DENSE_RAIL_SLOT_COUNT; slotIndex += 1) {
    const distanceFromActive = activeSlotIndex === undefined
      ? Number.POSITIVE_INFINITY
      : Math.abs(activeSlotIndex - slotIndex)
    context.fillStyle = slotIndex === activeSlotIndex
      ? 'rgb(39, 39, 42)'
      : emphasizedSlotIndexes.has(slotIndex)
        ? 'rgba(39, 39, 42, 0.55)'
        : 'rgba(113, 113, 122, 0.32)'
    context.fillRect(
      0,
      denseSlotTopPx(slotIndex),
      RAIL_MARKER_WIDTHS_PX[distanceFromActive] ?? RAIL_MARKER_DEFAULT_WIDTH_PX,
      RAIL_MARKER_HEIGHT_PX,
    )
  }
}

function activateDenseMarker(index: number, deferPreview = false) {
  denseActiveIndex.value = Math.min(props.markers.length - 1, Math.max(0, index))
  const marker = denseActiveMarker.value
  if (!marker) return
  activeMarkerId.value = marker.id
  if (densePreviewTimer !== null) clearTimeout(densePreviewTimer)
  if (deferPreview) {
    densePreviewTimer = setTimeout(() => {
      densePreviewTimer = null
      if (denseActiveMarker.value?.id === marker.id) openMarkerCard(marker)
    }, 80)
  } else {
    openMarkerCard(marker)
  }
}

function handleDensePointerMove(event: PointerEvent) {
  activateDenseMarker(denseMarkerIndex(event.clientY), true)
}

function handleDenseClick(event: MouseEvent) {
  const marker = props.markers[denseMarkerIndex(event.clientY)]
  if (marker) handleMarkerClick(marker)
}

function handleDenseKeydown(event: KeyboardEvent) {
  if (event.key === 'ArrowUp') {
    event.preventDefault()
    activateDenseMarker(denseActiveIndex.value - 1)
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    activateDenseMarker(denseActiveIndex.value + 1)
  } else if (event.key === 'Home') {
    event.preventDefault()
    activateDenseMarker(0)
  } else if (event.key === 'End') {
    event.preventDefault()
    activateDenseMarker(props.markers.length - 1)
  } else if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    const marker = denseActiveMarker.value
    if (marker && activeMarkerId.value === marker.id) handleMarkerClick(marker)
  }
}

function getScrollElement(): HTMLElement | null {
  const parent = props.scrollParent
  if (typeof HTMLElement !== 'undefined' && parent instanceof HTMLElement) {
    return parent
  }
  return null
}

function loadedChangedPaths(markerId: string): ChatUiRunChangedPathEntry[] | null {
  const state = changedPathsByMarkerId.value[markerId]
  return state?.status === 'loaded' ? state.paths : null
}

function hasCurrentMarker(markerId: string, runId: string): boolean {
  return markerIds.has(markerId) && loadedMarkerById.value.get(markerId)?.runId === runId
}

function bindTimelineIntersectionObserver() {
  unbindTimelineIntersectionObserver()
  const root = getScrollElement()
  const timelineRoot = props.timelineRoot
  if (!root || !timelineRoot || typeof IntersectionObserver === 'undefined') {
    return
  }

  markerIdsByRowKey.clear()
  for (const marker of props.loadedMarkers) {
    for (const rowKey of marker.visibleRowKeys) {
      const markerIdsForRow = markerIdsByRowKey.get(rowKey)
      if (markerIdsForRow) markerIdsForRow.push(marker.id)
      else markerIdsByRowKey.set(rowKey, [marker.id])
    }
  }

  timelineIntersectionObserver = new IntersectionObserver((entries) => {
    const nextNearMarkerIds = new Set(nearMarkerIds.value)
    for (const entry of entries) {
      const rowKey = (entry.target as HTMLElement).dataset.timelineRowKey
      if (!rowKey) {
        continue
      }
      const wasIntersecting = intersectingRowKeys.has(rowKey)
      if (wasIntersecting === entry.isIntersecting) continue
      if (entry.isIntersecting) {
        intersectingRowKeys.add(rowKey)
      } else {
        intersectingRowKeys.delete(rowKey)
      }
      for (const markerId of markerIdsByRowKey.get(rowKey) ?? []) {
        const count = Math.max(
          0,
          (nearMarkerRefCount.get(markerId) ?? 0) + (entry.isIntersecting ? 1 : -1),
        )
        if (count === 0) {
          nearMarkerRefCount.delete(markerId)
          nextNearMarkerIds.delete(markerId)
        } else {
          nearMarkerRefCount.set(markerId, count)
          nextNearMarkerIds.add(markerId)
        }
      }
    }
    if (!props.layoutResizing) {
      nearMarkerIds.value = nextNearMarkerIds
    }
  }, { root })

  for (const element of Array.from(timelineRoot.querySelectorAll<HTMLElement>('[data-timeline-row-key]'))) {
    const rowKey = element.dataset.timelineRowKey
    if (rowKey && markerIdsByRowKey.has(rowKey)) {
      timelineIntersectionObserver.observe(element)
    }
  }
}

function unbindTimelineIntersectionObserver() {
  timelineIntersectionObserver?.disconnect()
  timelineIntersectionObserver = null
  intersectingRowKeys.clear()
  markerIdsByRowKey.clear()
  nearMarkerRefCount.clear()
  nearMarkerIds.value = new Set()
}

async function loadChangedPaths(marker: TimelineUserMessageMarker): Promise<void> {
  const sessionId = props.sessionId
  const runId = marker.runId
  const readRunChangedPaths = props.readRunChangedPaths
  if (!sessionId || !runId || readRunChangedPaths === undefined) {
    return
  }
  const key = cacheKey(sessionId, runId)
  const cached = changedPathsCache.get(key)
  if (cached) {
    if (props.sessionId !== sessionId || !hasCurrentMarker(marker.id, runId)) {
      return
    }
    changedPathsByMarkerId.value = {
      ...changedPathsByMarkerId.value,
      [marker.id]: cached,
    }
    return
  }
  const loading: ChangedPathsState = { status: 'loading' }
  changedPathsCache.set(key, loading)
  changedPathsByMarkerId.value = {
    ...changedPathsByMarkerId.value,
    [marker.id]: loading,
  }
  try {
    // No anchor id: a steer marker shares the run id but is not the turn's
    // consumed opener, and the main fold only stamps `runId` on that opener.
    // The run filter already scopes the projection.
    const paths = await readRunChangedPaths({
      sessionId,
      runId,
      ...(marker.turnEndSequence === undefined ? {} : { throughSequence: marker.turnEndSequence }),
    })
    const state: ChangedPathsState = paths === null
      ? { status: 'unavailable' }
      : { status: 'loaded', paths }
    // A run that is not finalized yet returns null. Do not cache that as a
    // permanent answer: the next hover can observe the finalized run.
    if (state.status === 'loaded') {
      changedPathsCache.set(key, state)
    } else {
      changedPathsCache.delete(key)
    }
    if (props.sessionId !== sessionId || !hasCurrentMarker(marker.id, runId)) {
      return
    }
    changedPathsByMarkerId.value = {
      ...changedPathsByMarkerId.value,
      [marker.id]: state,
    }
  } catch {
    changedPathsCache.delete(key)
    if (props.sessionId !== sessionId || !hasCurrentMarker(marker.id, runId)) {
      return
    }
    const next = { ...changedPathsByMarkerId.value }
    delete next[marker.id]
    changedPathsByMarkerId.value = next
  }
}

function openMarkerCard(marker: TimelineUserMessageMarker, options: { respectSuppression?: boolean } = {}) {
  if (options.respectSuppression === true && suppressedMarkerHoverId === marker.id) {
    return
  }
  suppressedMarkerHoverId = null
  activeMarkerId.value = marker.id
  void loadMessagePreview(marker)
  void loadChangedPaths(marker)
}

function handleMarkerEnter(marker: TimelineUserMessageMarker) {
  openMarkerCard(marker, { respectSuppression: true })
}

function handleMarkerFocus(marker: TimelineUserMessageMarker) {
  openMarkerCard(marker)
}

function handleMarkerBlur() {
  suppressedMarkerHoverId = null
  closeCard()
}

function closeCard() {
  activeMarkerId.value = null
}

function handleMarkerClick(marker: TimelineUserMessageMarker) {
  suppressedMarkerHoverId = marker.id
  closeCard()
  emit('marker-click', marker)
}

function handleRailLeave() {
  if (densePreviewTimer !== null) {
    clearTimeout(densePreviewTimer)
    densePreviewTimer = null
  }
  suppressedMarkerHoverId = null
  closeCard()
}

watch(() => props.sessionId, () => {
  messagePreviewByMarkerId.value = {}
  assistantPreviewByMarkerId.value = {}
})

watch(
  () => [
    props.sessionId,
    props.scrollParent,
    props.timelineRoot,
    props.rowDomSignature,
    loadedMarkerMappingSignature.value,
  ] as const,
  () => {
    nearMarkerIds.value = new Set()
    bindTimelineIntersectionObserver()
  },
  { immediate: true, flush: 'post' },
)

watch(
  markerStructureSignature,
  () => {
    markerOrdinalById.clear()
    markerIds.clear()
    props.markers.forEach((marker, index) => {
      markerOrdinalById.set(marker.userMessageId, index)
      markerIds.add(marker.id)
    })
    if (denseRail.value && activeMarkerId.value) {
      const activeMarkerIndex = props.markers.findIndex(marker => marker.id === activeMarkerId.value)
      if (activeMarkerIndex >= 0) {
        denseActiveIndex.value = activeMarkerIndex
      } else {
        activeMarkerId.value = null
      }
    }
    denseActiveIndex.value = Math.min(
      denseActiveIndex.value,
      Math.max(0, props.markers.length - 1),
    )
    if (activeMarkerId.value && !markerIds.has(activeMarkerId.value)) {
      activeMarkerId.value = null
    }
    void nextTick(drawDenseRail)
  },
  { immediate: true, flush: 'post' },
)

watch(
  [nearMarkerIds, activeMarkerId, denseActiveIndex],
  () => void nextTick(drawDenseRail),
  { flush: 'post' },
)

watch(denseRail, () => {
  if (densePreviewTimer !== null) {
    clearTimeout(densePreviewTimer)
    densePreviewTimer = null
  }
  suppressedMarkerHoverId = null
  activeMarkerId.value = null
})

watch(() => props.layoutResizing, resizing => {
  if (!resizing) {
    nearMarkerIds.value = new Set(nearMarkerRefCount.keys())
  }
})

watch(railHeightPx, () => void nextTick(drawDenseRail), { flush: 'post' })

onBeforeUnmount(() => {
  if (densePreviewTimer !== null) clearTimeout(densePreviewTimer)
  unbindTimelineIntersectionObserver()
  // Hover lookups are active-timeline scratch state. Drop them when the rail
  // leaves the screen so switching threads cannot retain another Session's
  // previews or changed-path lists.
  changedPathsCache.clear()
  messagePreviewCache.clear()
  messagePreviewLoadingKeys.clear()
})
</script>

<template>
  <div
    class="timeline-user-message-rail"
    :style="{
      '--timeline-user-message-rail-height': `${railHeightPx}px`,
      '--timeline-user-message-marker-hit-height': `${railMarkerPitchPx}px`,
    }"
    data-testid="timeline-user-message-rail"
    @pointerleave="handleRailLeave"
  >
    <div class="timeline-user-message-rail__markers">
      <template v-if="denseRail">
        <canvas
          ref="denseCanvasRef"
          class="timeline-user-message-rail__dense-canvas"
          aria-hidden="true"
        />
        <div
          role="listbox"
          tabindex="0"
          class="timeline-user-message-rail__dense-hitbox"
          :aria-label="props.labels.timeline(markers.length)"
          aria-orientation="vertical"
          :aria-activedescendant="activeMarkerId && denseActiveMarker ? `timeline-user-marker-${denseActiveMarker.userMessageId}` : undefined"
          :aria-owns="activeMarkerId && denseActiveMarker ? `timeline-user-marker-${denseActiveMarker.userMessageId}` : undefined"
          @focus="activateDenseMarker(denseActiveIndex)"
          @blur="handleMarkerBlur"
          @pointermove="handleDensePointerMove"
          @click="handleDenseClick"
          @keydown="handleDenseKeydown"
        ></div>
        <div
          v-if="denseActiveMarker && activeMarkerId === denseActiveMarker.id"
          :id="`timeline-user-marker-${denseActiveMarker.userMessageId}`"
          role="option"
          aria-selected="true"
          :aria-posinset="denseActiveIndex + 1"
          :aria-setsize="markers.length"
          :aria-label="props.labels.option(denseActiveIndex + 1, markers.length, markerPreview(denseActiveMarker))"
          class="timeline-user-message-rail__marker-wrap"
          :style="{ top: denseActiveTop }"
        >
          <div class="timeline-user-message-rail__card" data-testid="timeline-user-message-card">
            <div class="timeline-user-message-rail__preview">
              {{ markerPreview(denseActiveMarker) }}
            </div>
            <div
              v-if="markerAssistantPreview(denseActiveMarker)"
              class="timeline-user-message-rail__assistant-preview"
            >
              {{ markerAssistantPreview(denseActiveMarker) }}
            </div>
            <template v-if="loadedChangedPaths(denseActiveMarker.id) !== null">
              <div
                v-if="loadedChangedPaths(denseActiveMarker.id)?.length === 0"
                class="timeline-user-message-rail__empty"
              >
                {{ props.labels.emptyChangedPaths }}
              </div>
              <div v-else class="timeline-user-message-rail__chips">
                <div
                  v-for="entry in loadedChangedPaths(denseActiveMarker.id)?.slice(0, 4) ?? []"
                  :key="entry.path"
                  class="timeline-user-message-rail__chip"
                  :title="entry.path"
                >
                  <span
                    v-if="entry.changeType && entry.changeType !== 'unknown'"
                    class="timeline-user-message-rail__change-type"
                  >
                    {{ entry.changeType }}
                  </span>
                  <span class="timeline-user-message-rail__path">
                    {{ basename(entry.path) }}
                  </span>
                </div>
                <span
                  v-if="(loadedChangedPaths(denseActiveMarker.id)?.length ?? 0) > 4"
                  class="timeline-user-message-rail__more"
                >
                  +{{ (loadedChangedPaths(denseActiveMarker.id)?.length ?? 0) - 4 }}
                </span>
              </div>
            </template>
          </div>
        </div>
      </template>
      <div
        v-else
        v-for="(marker, index) in displayMarkers"
        :key="marker.id"
        class="timeline-user-message-rail__marker-wrap"
        :style="{ top: markerTopPx(index) }"
        @pointerenter="handleMarkerEnter(marker)"
        @pointerleave="closeCard"
      >
        <button
          type="button"
          class="timeline-user-message-rail__marker"
          :style="{ '--timeline-user-message-marker-width': `${markerWidthPx(index)}px` }"
          :class="{
            'timeline-user-message-rail__marker--active': activeMarkerId === marker.id,
            'timeline-user-message-rail__marker--near': nearMarkerIds.has(marker.id),
          }"
          :aria-label="props.labels.jumpToMessage(markerPreview(marker))"
          :data-timeline-user-message-marker-id="marker.userMessageId"
          @focus="handleMarkerFocus(marker)"
          @blur="handleMarkerBlur"
          @click="handleMarkerClick(marker)"
        >
          <span />
        </button>

        <div
          v-if="activeMarkerId === marker.id"
          class="timeline-user-message-rail__card"
          data-testid="timeline-user-message-card"
          @pointerdown="closeCard"
        >
          <div class="timeline-user-message-rail__preview">
              {{ markerPreview(marker) }}
          </div>
          <div
            v-if="markerAssistantPreview(marker)"
            class="timeline-user-message-rail__assistant-preview"
          >
            {{ markerAssistantPreview(marker) }}
          </div>

          <template v-if="loadedChangedPaths(marker.id) !== null">
            <div
              v-if="loadedChangedPaths(marker.id)?.length === 0"
              class="timeline-user-message-rail__empty"
            >
              {{ props.labels.emptyChangedPaths }}
            </div>
            <div
              v-else
              class="timeline-user-message-rail__chips"
            >
              <div
                v-for="entry in loadedChangedPaths(marker.id)?.slice(0, 4) ?? []"
                :key="entry.path"
                class="timeline-user-message-rail__chip"
                :title="entry.path"
              >
                <span
                  v-if="entry.changeType && entry.changeType !== 'unknown'"
                  class="timeline-user-message-rail__change-type"
                >
                  {{ entry.changeType }}
                </span>
                <span class="timeline-user-message-rail__path">
                  {{ basename(entry.path) }}
                </span>
              </div>
              <span
                v-if="(loadedChangedPaths(marker.id)?.length ?? 0) > 4"
                class="timeline-user-message-rail__more"
              >
                +{{ (loadedChangedPaths(marker.id)?.length ?? 0) - 4 }}
              </span>
            </div>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline-user-message-rail {
  pointer-events: auto;
  width: 26px;
  transform: translateY(-50%);
}

.timeline-user-message-rail__markers {
  pointer-events: auto;
  position: relative;
  height: var(--timeline-user-message-rail-height);
}

.timeline-user-message-rail__dense-canvas,
.timeline-user-message-rail__dense-hitbox {
  position: absolute;
  inset: 0;
  width: 26px;
  height: var(--timeline-user-message-rail-height);
}

.timeline-user-message-rail__dense-canvas {
  pointer-events: none;
}

.timeline-user-message-rail__dense-hitbox {
  z-index: 1;
  border: 0;
  background: transparent;
  cursor: pointer;
}

.timeline-user-message-rail__dense-hitbox:focus-visible {
  outline: 1px solid hsl(var(--ring));
  outline-offset: 2px;
}

.timeline-user-message-rail__marker-wrap {
  position: absolute;
  left: 0;
  display: flex;
  width: 26px;
  height: var(--timeline-user-message-marker-hit-height);
  align-items: center;
  transform: translateY(-50%);
}

.timeline-user-message-rail__marker {
  display: flex;
  width: 26px;
  height: 100%;
  align-items: center;
  border: 0;
  background: transparent;
  padding: 0;
  cursor: pointer;
}

.timeline-user-message-rail__marker:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
  border-radius: 9999px;
}

.timeline-user-message-rail__marker > span {
  display: block;
  width: var(--timeline-user-message-marker-width, 6px);
  height: 2px;
  border-radius: 9999px;
  background: color-mix(in oklab, var(--muted-foreground) 32%, transparent);
  transition: width 150ms ease-out, background-color 150ms ease-out;
}

.timeline-user-message-rail__marker--near > span {
  background: color-mix(in oklab, var(--foreground) 55%, transparent);
}

.timeline-user-message-rail__marker--active > span {
  background: var(--foreground);
}

.timeline-user-message-rail__card {
  pointer-events: auto;
  position: absolute;
  top: 50%;
  left: 30px;
  z-index: 40;
  width: min(360px, calc(100vw - 96px));
  transform: translateY(-50%);
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in oklab, var(--background) 96%, transparent);
  box-shadow: 0 14px 40px oklch(0 0 0 / 12%);
  padding: 12px 14px;
}

.timeline-user-message-rail__preview {
  display: -webkit-box;
  overflow: hidden;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  color: var(--foreground);
  font-size: 14px;
  font-weight: 600;
  line-height: 1.45;
}

.timeline-user-message-rail__assistant-preview {
  display: -webkit-box;
  overflow: hidden;
  margin-top: 6px;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  color: var(--muted-foreground);
  font-size: 13px;
  font-weight: 400;
  line-height: 1.4;
}

.timeline-user-message-rail__empty {
  margin-top: 8px;
  color: var(--muted-foreground);
  font-size: 13px;
  line-height: 1.4;
}

.timeline-user-message-rail__chips {
  display: flex;
  min-width: 0;
  margin-top: 10px;
  gap: 8px;
  align-items: center;
}

.timeline-user-message-rail__chip {
  display: flex;
  min-width: 0;
  max-width: 132px;
  align-items: center;
  gap: 5px;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1;
}

.timeline-user-message-rail__change-type {
  flex: 0 0 auto;
  border-radius: 4px;
  background: color-mix(in oklab, var(--muted-foreground) 16%, transparent);
  padding: 2px 4px;
  color: var(--muted-foreground);
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
}

.timeline-user-message-rail__path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.timeline-user-message-rail__more {
  flex: 0 0 auto;
  color: var(--muted-foreground);
  font-size: 13px;
  line-height: 1;
}
</style>
