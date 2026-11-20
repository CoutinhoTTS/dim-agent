import { defineAsyncComponent, h } from "vue"

function readFlag(): boolean {
  try {
    return new URLSearchParams(window.location.search).get("chatUi") === "1" || window.localStorage.getItem("dim.chatUi") === "1"
  } catch {
    return false
  }
}

/** The shared timeline replaces `TimelineView` when the page opened with `?chatUi=1` or `localStorage["dim.chatUi"] === "1"`. */
export const chatUiEnabled = readFlag()

/** `WebChatTimeline` in its own chunk; the shell it fills keeps the grid row while the chunk loads. */
export const WebChatTimeline = defineAsyncComponent({
  loader: () => import("./WebChatTimeline.vue"),
  loadingComponent: { render: () => h("div", { class: "timeline-shell" }) },
  delay: 0,
})
