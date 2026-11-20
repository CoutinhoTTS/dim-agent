import { defineComponent, h, inject, type Component, type ComputedRef, type InjectionKey, type Ref } from "vue"
import type { TimelineRow as ProjectionRow } from "../projection"
import type { WebTimelineLabels } from "./components"
import WebIcon, { ICON } from "./WebIcon.vue"

/** What the Web renderers read from the timeline that mounts them. */
export type WebTimelineContext = {
  rowByKey: (key: string) => ProjectionRow | undefined
  isDark: Readonly<Ref<boolean>>
  labels: ComputedRef<WebTimelineLabels>
  openLine: (branchId: string) => void
}

export const WEB_TIMELINE: InjectionKey<WebTimelineContext> = Symbol("web-timeline")

export function useWebTimeline(): WebTimelineContext {
  const context = inject(WEB_TIMELINE)
  if (context === undefined) throw new Error("WebChatTimeline must provide WEB_TIMELINE")
  return context
}

export function icon(name: string): Component {
  return defineComponent({
    name: "WebChatIcon",
    inheritAttrs: false,
    setup: (_, { attrs }) => () => h(WebIcon, { ...attrs, name }),
  })
}

export const icons = {
  chevronRight: icon(ICON.chevronRight),
  chevronDown: icon(ICON.chevronDown),
  chevronUp: icon(ICON.chevronUp),
  copy: icon(ICON.copy),
  check: icon(ICON.check),
  gitFork: icon(ICON.gitFork),
  gitBranch: icon(ICON.gitBranch),
  retry: icon(ICON.rotateCcw),
}

/** The package's `button` slots: a plain button with the inline layout the package's classes assume. */
export const Button = defineComponent({
  name: "WebChatButton",
  props: { variant: String, size: String },
  setup: (_, { slots }) => () => h("button", { type: "button", class: "inline-flex items-center whitespace-nowrap" }, slots.default?.()),
})

/** Tooltip and menu roots and triggers render their child as it is. */
export const Passthrough = defineComponent({
  name: "WebChatPassthrough",
  inheritAttrs: false,
  setup: (_, { slots }) => () => slots.default?.(),
})

/** Surfaces the read-only Web timeline does not offer: tooltip and menu content, MCP Apps, provider icons. */
export const Empty = defineComponent({ name: "WebChatEmpty", inheritAttrs: false, setup: () => () => null })

const FILE_ICON = `data:image/svg+xml,${encodeURIComponent("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='gray' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z'/><path d='M14 2v4a2 2 0 0 0 2 2h4'/></svg>")}`

export function fileIconUrl(): string {
  return FILE_ICON
}
