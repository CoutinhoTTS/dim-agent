import { describe, expect, it } from 'vitest'
import {
  TIMELINE_HISTORY_COLLAPSE_COMPLETED_TURN_THRESHOLD,
  calculateTimelineWorkDuration,
  isTimelineWorkBlock,
  resolveTimelineBlockExpansionId,
  resolveTimelineTurnExpansionId,
  shouldEnableTimelineTurnDetailsCollapse,
  shouldExpandTimelineTurnDetails,
  shouldRenderTimelineTurnWorkDetails,
} from '../src'

const collapseContext = {
  turnIndex: 0,
  turnCount: 10,
  completedTurnCount: TIMELINE_HISTORY_COLLAPSE_COMPLETED_TURN_THRESHOLD + 1,
  latestCompletedTurnIndex: 9,
  hasRunningTurn: false,
}

describe('timeline turn state', () => {
  it('resolves expansion ids from turn and block ids', () => {
    expect(resolveTimelineTurnExpansionId({ id: 'turn-1' })).toBe('turn-1')
    expect(resolveTimelineBlockExpansionId({ id: 'block-1' })).toBe('block-1')
  })

  it('expands running turns and collapses only older completed turns in a long history', () => {
    expect(shouldExpandTimelineTurnDetails({ isRunning: true }, collapseContext)).toBe(true)
    expect(shouldEnableTimelineTurnDetailsCollapse({ isRunning: true }, collapseContext)).toBe(false)

    expect(shouldEnableTimelineTurnDetailsCollapse({ isRunning: false }, collapseContext)).toBe(true)
    expect(shouldExpandTimelineTurnDetails({ isRunning: false }, collapseContext)).toBe(false)

    expect(shouldExpandTimelineTurnDetails({ isRunning: false }, {
      ...collapseContext,
      completedTurnCount: TIMELINE_HISTORY_COLLAPSE_COMPLETED_TURN_THRESHOLD,
    })).toBe(true)
    expect(shouldExpandTimelineTurnDetails({ isRunning: false }, undefined)).toBe(true)
  })

  it('keeps the latest completed turn open when nothing is running', () => {
    expect(shouldEnableTimelineTurnDetailsCollapse({ isRunning: false }, {
      ...collapseContext,
      turnIndex: collapseContext.latestCompletedTurnIndex,
    })).toBe(false)
    expect(shouldEnableTimelineTurnDetailsCollapse({ isRunning: false }, {
      ...collapseContext,
      hasRunningTurn: true,
    })).toBe(true)
  })

  it('classifies work blocks and detects work details', () => {
    expect(isTimelineWorkBlock({ kind: 'thinking' })).toBe(true)
    expect(isTimelineWorkBlock({ kind: 'tool' })).toBe(true)
    expect(isTimelineWorkBlock({ kind: 'notification' })).toBe(true)
    expect(isTimelineWorkBlock({ kind: 'text' })).toBe(false)
    expect(isTimelineWorkBlock({ kind: 'status' })).toBe(false)
    expect(isTimelineWorkBlock({ kind: 'generated-image' })).toBe(false)
    expect(shouldRenderTimelineTurnWorkDetails({
      blocks: [{ kind: 'text' }, { kind: 'tool' }],
    })).toBe(true)
    expect(shouldRenderTimelineTurnWorkDetails({ blocks: [{ kind: 'text' }] })).toBe(false)
  })

  it('calculates work duration only when every work block is timed', () => {
    expect(calculateTimelineWorkDuration([
      { kind: 'thinking', startedAt: new Date(1000), endedAt: new Date(5000) },
      { kind: 'tool', startedAt: new Date(3000), endedAt: new Date(9000) },
      { kind: 'text', startedAt: null, endedAt: null },
    ])).toBe(8000)

    expect(calculateTimelineWorkDuration([
      { kind: 'tool', startedAt: new Date(1000), endedAt: new Date(5000) },
      { kind: 'tool', startedAt: null, endedAt: null },
    ])).toBeNull()
    expect(calculateTimelineWorkDuration([{ kind: 'text' }])).toBeNull()
    expect(calculateTimelineWorkDuration([])).toBeNull()
  })
})
