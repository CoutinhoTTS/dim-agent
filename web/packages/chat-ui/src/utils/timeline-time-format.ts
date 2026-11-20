export interface FormatTimelineTimestampOptions {
  /** Render the time of day only. Used by LatestTurnPreview. */
  timeOnly?: boolean
}

const DAY_MS = 24 * 60 * 60 * 1000

const startOfLocalDay = (date: Date): number => {
  const copy = new Date(date)
  copy.setHours(0, 0, 0, 0)
  return copy.getTime()
}

const timeOfDayFormatter = (locale: string): Intl.DateTimeFormat =>
  new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hour12: false })

/**
 * Format a timeline timestamp with a date tier chosen by distance from `now`:
 *
 * - < 7 local calendar days: weekday + time (`星期三 14:30`)
 * - >= 7 days, same year: month + day + time (`3月19日 14:30`)
 * - >= 7 days, different year: year + month + day + time (`2025年3月19日 14:30`)
 *
 * Times are 24-hour and the date/time parts are formatted separately so output
 * does not depend on the host's combined ICU patterns. An invalid `at` throws
 * `RangeError` from `Intl.DateTimeFormat.format`.
 */
export const formatTimelineTimestamp = (
  at: Date,
  locale: string,
  now: Date,
  options: FormatTimelineTimestampOptions = {},
): string => {
  const timePart = timeOfDayFormatter(locale).format(at)
  if (options.timeOnly) {
    return timePart
  }

  const dayDiffMs = startOfLocalDay(now) - startOfLocalDay(at)
  if (dayDiffMs < 7 * DAY_MS) {
    const weekdayPart = new Intl.DateTimeFormat(locale, { weekday: 'long' }).format(at)
    return `${weekdayPart} ${timePart}`
  }

  const sameYear = at.getFullYear() === now.getFullYear()
  const datePart = new Intl.DateTimeFormat(locale, {
    ...(sameYear ? {} : { year: 'numeric' }),
    month: 'short',
    day: 'numeric',
  }).format(at)
  return `${datePart} ${timePart}`
}
