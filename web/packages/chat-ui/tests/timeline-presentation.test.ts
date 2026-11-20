import { describe, expect, it } from 'vitest'
import {
  CHAT_UI_TIMELINE_BRANCH_ID,
  CHAT_UI_TIMELINE_DETAIL_SESSION_ID,
  CHAT_UI_TIMELINE_RESTORE_ACTIVE,
} from '../src'

describe('timeline scope keys', () => {
  it('exposes one distinct injection key per timeline scope', () => {
    const keys = [
      CHAT_UI_TIMELINE_RESTORE_ACTIVE,
      CHAT_UI_TIMELINE_BRANCH_ID,
      CHAT_UI_TIMELINE_DETAIL_SESSION_ID,
    ]

    expect(keys.every(key => typeof key === 'symbol')).toBe(true)
    expect(new Set(keys).size).toBe(keys.length)
  })
})
