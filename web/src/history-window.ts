import type { HistoryPage, SessionEvent } from "./rpc/protocol"
import { ProtocolError } from "./rpc/read"

function identities(events: readonly SessionEvent[]): Set<string> {
  const rows = new Set<string>()
  for (const event of events) {
    const content = event.content
    switch (content.kind) {
      case "input": rows.add(`input:${event.id}`); break
      case "text": if (content.delta !== "") rows.add(`reply:${content.requestId}`); break
      case "tool-use": rows.add(`tool:${content.requestId}:${content.callId}`); break
      case "tool-completed": rows.add(`tool:${content.requestId}:${content.callId}`); break
      case "tool-result": for (const result of content.results) rows.add(`tool:${content.requestId}:${result.callId}`); break
    }
  }
  return rows
}

/** Raw pages belong only to this load and are released after projection. */
export class HistoryWindow {
  private pages: HistoryPage[] = []
  through: number | undefined
  before: number | undefined

  constructor(before?: number, through?: number) {
    this.before = before
    this.through = through
  }

  params(sessionId: string, branchId: string): Record<string, unknown> {
    return { sessionId, branchId, limit: 256, ...(this.before === undefined
      ? { latestTurn: true }
      : { before: this.before, through: this.through }) }
  }

  push(page: HistoryPage): boolean {
    if ((this.through !== undefined && page.through !== this.through)
      || (page.nextBefore !== undefined && (page.nextBefore < 1 || page.nextBefore > page.through || (this.before !== undefined && page.nextBefore >= this.before)))
      || page.events.some(event => event.sequence > page.through || (this.before !== undefined && event.sequence >= this.before))) {
      throw new ProtocolError("Invalid history window cursor")
    }
    this.through = page.through
    this.before = page.nextBefore !== undefined && page.nextBefore > 1 ? page.nextBefore : undefined
    this.pages.unshift(page)
    return this.before === undefined || this.pages.length >= 4 || identities(this.pages.flatMap(page => page.events)).size >= 100
  }

  take(): SessionEvent[] {
    const rows = new Set<string>()
    let keep = 0
    for (const page of [...this.pages].reverse()) {
      const before = rows.size
      for (const id of identities(page.events)) rows.add(id)
      if (before > 0 && rows.size > 300) break
      keep++
    }
    this.pages.splice(0, this.pages.length - keep)
    const before = this.pages[0]?.nextBefore
    this.before = before !== undefined && before > 1 ? before : undefined
    const events = this.pages.flatMap(page => page.events)
    this.pages = []
    return events
  }
}
