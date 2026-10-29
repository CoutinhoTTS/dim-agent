/**
 * The Session's lines, folded from the branch facts in every branch's history.
 *
 * A line is a Timeline of the same Session. It has one parent, a cut in that
 * parent, and a purpose from its opening input. This registry owns the tree,
 * the cut's user-message count and the line state; the transcript stays in
 * `Projection`.
 */
import type { SessionEvent } from "./rpc/protocol"

export type LineState =
  | { kind: "main" }
  | { kind: "working" }
  | { kind: "waiting" }
  | { kind: "ended-itself" }
  | { kind: "ended-by-operation" }
  | { kind: "ended-with"; ancestor: string }
  | { kind: "creation-failed" }

export type LineView = {
  id: string
  parentId: string | null
  /** The first line of the purpose the line opened with. */
  purpose: string
  state: LineState
  /** Where it was cut from its parent; `null` for main. */
  cut: { sequence: number; messages: number } | null
  depth: number
}

type LineNode = {
  id: string
  parentId: string
  purpose: string
  at: number
  order: number
  failed: boolean
  ended: LineState | undefined
}

const endsItself = { kind: "ended-itself" } as const
const endsByOperation = { kind: "ended-by-operation" } as const

export class Lines {
  private nodes = new Map<string, LineNode>()
  private messages = new Map<string, Map<string, number>>()
  private running = new Set<string>()
  private nextOrder = 0

  /** Folds one event of `branchId` into the tree. Repeated facts are idempotent. */
  apply(branchId: string, event: SessionEvent): void {
    const content = event.content
    switch (content.kind) {
      case "input":
        this.addMessage(branchId, event.id, event.sequence)
        break
      case "input-deleted":
        this.messages.get(branchId)?.delete(content.inputId)
        break
      case "branch-create": {
        if (this.nodes.has(content.branchId)) return
        this.nodes.set(content.branchId, {
          id: content.branchId,
          parentId: branchId,
          purpose: firstLine(content.prompt),
          at: content.at ?? event.sequence,
          order: this.nextOrder++,
          failed: false,
          ended: undefined,
        })
        break
      }
      case "branch-create-failed": {
        const node = this.nodes.get(content.branchId) ?? {
          id: content.branchId,
          parentId: branchId,
          purpose: "",
          at: event.sequence,
          order: this.nextOrder++,
          failed: false,
          ended: undefined,
        }
        node.failed = true
        this.nodes.set(node.id, node)
        break
      }
      case "branch-ended": {
        const node = this.nodes.get(content.branchId)
        if (node !== undefined) node.ended = endedState(content.source, content.ancestor)
        break
      }
      default:
        break
    }
  }

  setRunning(branchId: string, running: boolean): void {
    if (running) this.running.add(branchId)
    else this.running.delete(branchId)
  }

  has(branchId: string): boolean {
    return this.nodes.has(branchId)
  }

  /** Every line in tree order: main first, then children by cut and opening order. */
  views(): LineView[] {
    const out: LineView[] = [
      { id: "main", parentId: null, purpose: "", state: { kind: "main" }, cut: null, depth: 0 },
    ]
    const children = new Map<string, LineNode[]>()
    for (const node of this.nodes.values()) {
      const siblings = children.get(node.parentId) ?? []
      siblings.push(node)
      children.set(node.parentId, siblings)
    }
    for (const siblings of children.values()) {
      siblings.sort((left, right) => left.at - right.at || left.order - right.order)
    }
    const seen = new Set<string>()
    const walk = (parent: string, depth: number): void => {
      for (const node of children.get(parent) ?? []) {
        if (seen.has(node.id)) continue
        seen.add(node.id)
        out.push({
          id: node.id,
          parentId: node.parentId,
          purpose: node.purpose,
          state: this.stateOf(node),
          cut: { sequence: node.at, messages: this.cutMessages(node) },
          depth,
        })
        walk(node.id, depth + 1)
      }
    }
    walk("main", 1)
    for (const node of this.nodes.values()) {
      if (seen.has(node.id)) continue
      seen.add(node.id)
      out.push({
        id: node.id,
        parentId: node.parentId,
        purpose: node.purpose,
        state: this.stateOf(node),
        cut: { sequence: node.at, messages: this.cutMessages(node) },
        depth: 1,
      })
      walk(node.id, 2)
    }
    return out
  }

  private addMessage(branchId: string, inputId: string, sequence: number): void {
    const messages = this.messages.get(branchId) ?? new Map<string, number>()
    messages.set(inputId, sequence)
    this.messages.set(branchId, messages)
  }

  private stateOf(node: LineNode): LineState {
    if (node.failed) return { kind: "creation-failed" }
    if (node.ended !== undefined) return node.ended
    return this.running.has(node.id) ? { kind: "working" } : { kind: "waiting" }
  }

  private cutMessages(node: LineNode): number {
    const parent = this.nodes.get(node.parentId)
    const inherited = parent === undefined ? 0 : this.cutMessages(parent)
    return inherited + this.messagesThrough(node.parentId, node.at)
  }

  private messagesThrough(branchId: string, sequence: number): number {
    let count = 0
    for (const at of this.messages.get(branchId)?.values() ?? []) {
      if (at <= sequence) count++
    }
    return count
  }
}

function firstLine(text: string): string {
  return text.split("\n")[0]?.trim() ?? ""
}

function endedState(source: { kind: string }, ancestor: string | undefined): LineState {
  if (ancestor !== undefined) return { kind: "ended-with", ancestor }
  return source.kind === "tool" ? endsItself : endsByOperation
}
