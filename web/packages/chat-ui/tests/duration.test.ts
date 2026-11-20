import { describe, expect, it } from 'vitest'
import {
  calculateMessageWorkDuration,
  formatElapsedSeconds,
  formatWorkDuration,
} from '../src'

describe('timeline duration formatting', () => {
  it('formats work durations across second, minute, and hour ranges', () => {
    expect(formatWorkDuration(0)).toBe('0.0s')
    expect(formatWorkDuration(1500)).toBe('1.5s')
    expect(formatWorkDuration(59_900)).toBe('59.9s')
    expect(formatWorkDuration(60_000)).toBe('1m 0s')
    expect(formatWorkDuration(125_000)).toBe('2m 5s')
    expect(formatWorkDuration(3_600_000)).toBe('1h 0m')
    expect(formatWorkDuration(5_460_000)).toBe('1h 31m')
    expect(formatWorkDuration(-10)).toBe('0.0s')
  })

  it('formats elapsed seconds with a sub-second decimal', () => {
    expect(formatElapsedSeconds(0)).toBe('0s')
    expect(formatElapsedSeconds(400)).toBe('0.4s')
    expect(formatElapsedSeconds(15_900)).toBe('15s')
    expect(formatElapsedSeconds(60_000)).toBe('1m')
    expect(formatElapsedSeconds(95_000)).toBe('1m 35s')
    expect(formatElapsedSeconds(-10)).toBe('0s')
  })

  it('calculates message work duration from non-text events', () => {
    const updatedAt = new Date(10_000)
    expect(calculateMessageWorkDuration({
      updatedAt,
      events: [
        { id: '1', type: 'thinking', status: 'complete', startTime: 1000, endTime: 3000 },
        { id: '2', type: 'tool_call', status: 'complete', startTime: 2000, endTime: 7000 },
        { id: '3', type: 'text_delta', status: 'complete', startTime: 0, endTime: 10_000 },
      ],
    })).toBe(6000)

    expect(calculateMessageWorkDuration({
      updatedAt,
      events: [{ id: '1', type: 'tool_call', status: 'complete', startTime: 2000 }],
    })).toBe(8000)

    expect(calculateMessageWorkDuration({ updatedAt, events: [] })).toBe(0)
    expect(calculateMessageWorkDuration({ updatedAt })).toBe(0)
    expect(calculateMessageWorkDuration({
      updatedAt,
      events: [{ id: '1', type: 'text_delta', status: 'complete', startTime: 0, endTime: 1000 }],
    })).toBe(0)
  })
})
