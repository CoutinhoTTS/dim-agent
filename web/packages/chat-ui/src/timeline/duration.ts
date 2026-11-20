import type { ChatUiMessage, ChatUiMessageEvent } from '../model/chat-message'

export function formatWorkDuration(durationMs: number): string {
  const safeDurationMs = Math.max(durationMs, 0)

  if (safeDurationMs < 60_000) {
    return `${(safeDurationMs / 1000).toFixed(1)}s`
  }

  if (safeDurationMs < 3_600_000) {
    const minutes = Math.floor(safeDurationMs / 60_000)
    const seconds = Math.round((safeDurationMs % 60_000) / 1000)
    return `${minutes}m ${seconds}s`
  }

  const hours = Math.floor(safeDurationMs / 3_600_000)
  const minutes = Math.round((safeDurationMs % 3_600_000) / 60_000)
  return `${hours}h ${minutes}m`
}

export function formatElapsedSeconds(durationMs: number): string {
  const safeDurationMs = Math.max(durationMs, 0)
  const totalSeconds = Math.floor(safeDurationMs / 1000)
  if (totalSeconds < 60) {
    // A sub-second duration used to floor to "0s", which reads as a display
    // bug; show one decimal for it (e.g. "0.4s"), keep whole seconds as-is.
    return totalSeconds === 0 && safeDurationMs > 0
      ? `${Math.round(safeDurationMs / 100) / 10}s`
      : `${totalSeconds}s`
  }

  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`
}

export function calculateMessageWorkDuration(
  message: Pick<ChatUiMessage, 'events' | 'updatedAt'>,
): number {
  if (!message.events || message.events.length === 0) {
    return 0
  }

  const workEvents = message.events.filter((event: ChatUiMessageEvent) => event.type !== 'text_delta')
  if (workEvents.length === 0) {
    return 0
  }

  const earliestStart = Math.min(...workEvents.map(event => event.startTime))
  const latestEnd = Math.max(...workEvents.map(event => event.endTime ?? message.updatedAt.getTime()))
  return Math.max(0, latestEnd - earliestStart)
}
