import type { LineState, LineView } from "../../lines"

export type Translate = (key: string, params?: Record<string, unknown>) => string

export function shortId(id: string): string {
  return id.slice(0, 8)
}

export function lineStateLabel(t: Translate, state: LineState): string {
  switch (state.kind) {
    case "main": return t("lines.state.main")
    case "working": return t("lines.state.working")
    case "waiting": return t("lines.state.waiting")
    case "ended-itself": return t("lines.state.endedItself")
    case "ended-by-operation": return t("lines.state.endedByOperation")
    case "ended-with": return t("lines.state.endedWith", { id: state.ancestor })
    case "creation-failed": return t("lines.state.creationFailed")
  }
}

export function lineStateTone(state: LineState): "primary" | "warning" | "danger" | undefined {
  if (state.kind === "working") return "primary"
  if (state.kind === "creation-failed") return "danger"
  if (state.kind === "ended-with") return "warning"
  return undefined
}

export function lineName(t: Translate, line: LineView | undefined, branchId: string): string {
  const purpose = line?.purpose.trim() ?? ""
  return purpose === "" ? shortId(branchId) : purpose
}
