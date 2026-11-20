import type {
  ChatUiToolRenderBlock,
  ToolCallRenderer,
  TimelineToolInlineParameter,
  TimelineToolDetailSection,
} from './types'
import type { ChatUiToolLabelKey } from './keys'
import {
  asRecord,
  normalizeToolName,
  readParamsRecord,
  readString,
  readablePairLines,
  resultText,
  previewLinesFromText,
  truncateNonEmptyToolPreviewText,
  summarySection,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

/**
 * Browser tool renderer. The `browser` tool is a 15-action discriminated union
 * (see packages/plugin/src/builtin/browser.ts). The collapsed row shows the
 * action + its key argument; actions with sizeable output (snapshot, console,
 * errors, eval, cdp) expand to show the result. `screenshot` renders no detail
 * here — its thumbnail is a companion image block (see image-generation-result).
 */

/** Per-action primary-parameter i18n key + the param read for its `{...}` slot. */
const ACTION_PRIMARY: Record<
  string,
  { key: ChatUiToolLabelKey; param?: 'url' | 'ref' | 'key' | 'direction' | 'type' | 'method' }
> = {
  navigate: { key: 'toolBrowser.navigate', param: 'url' },
  reload: { key: 'toolBrowser.reload' },
  snapshot: { key: 'toolBrowser.snapshot' },
  screenshot: { key: 'toolBrowser.screenshot' },
  console: { key: 'toolBrowser.console' },
  errors: { key: 'toolBrowser.errors' },
  click: { key: 'toolBrowser.click', param: 'ref' },
  fill: { key: 'toolBrowser.fill', param: 'ref' },
  press: { key: 'toolBrowser.press', param: 'key' },
  scroll: { key: 'toolBrowser.scroll', param: 'direction' },
  wait: { key: 'toolBrowser.wait', param: 'type' },
  eval: { key: 'toolBrowser.eval' },
  cdp: { key: 'toolBrowser.cdp', param: 'method' },
}

/** Actions whose result is worth expanding into a detail section. */
const DETAIL_ACTIONS = new Set(['snapshot', 'console', 'errors', 'eval'])

interface BrowserParams {
  action: string
  url?: string
  ref?: string
  key?: string
  direction?: string
  type?: string
  method?: string
  params?: Record<string, unknown>
}

const readBrowserParams = (block: ChatUiToolRenderBlock): BrowserParams => {
  const p = readParamsRecord(block) ?? {}
  const action = typeof p.action === 'string' ? p.action.trim().toLowerCase() : ''
  return {
    action,
    url: readString(p, ['url']) ?? undefined,
    ref: readString(p, ['ref']) ?? undefined,
    key: readString(p, ['key']) ?? undefined,
    direction: readString(p, ['direction']) ?? undefined,
    type: readString(p, ['type']) ?? undefined,
    method: readString(p, ['method']) ?? undefined,
    params: asRecord(p.params) ?? undefined,
  }
}

const primaryParamValue = (
  bp: BrowserParams,
  param: 'url' | 'ref' | 'key' | 'direction' | 'type' | 'method' | undefined,
): string | undefined => {
  if (param === undefined) return undefined
  const raw = bp[param]
  if (raw === undefined) return undefined
  return truncateNonEmptyToolPreviewText(raw)
}

export const browserRenderer: ToolCallRenderer = {
  id: 'browser',

  match(toolName) {
    return normalizeToolName(toolName) === 'browser'
  },

  getLabelKey(): ChatUiToolLabelKey {
    return 'toolDisplay.browser'
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const bp = readBrowserParams(block)
    const spec = ACTION_PRIMARY[bp.action]
    if (spec === undefined) {
      return null
    }
    const value = primaryParamValue(bp, spec.param)
    return value !== undefined && spec.param !== undefined
      ? { kind: 'i18n', key: spec.key, params: { [spec.param]: value } }
      : { kind: 'i18n', key: spec.key }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const bp = readBrowserParams(block)

    if (bp.action === 'cdp') {
      const lines: string[] = [`method: ${bp.method ?? ''}`]
      if (bp.params && Object.keys(bp.params).length > 0) {
        lines.push(`params: ${JSON.stringify(bp.params)}`)
      }
      const text = resultText(block)
      if (text) {
        const preview = previewLinesFromText(text, DETAIL_LINE_MAX)
        if (preview) lines.push(...preview.lines)
      }
      const section = summarySection('browser-cdp', lines)
      return section ? [section] : []
    }

    if (DETAIL_ACTIONS.has(bp.action)) {
      const preview = previewLinesFromText(resultText(block), DETAIL_LINE_MAX)
      return preview ? [{ ...preview, id: `browser-${bp.action}` }] : []
    }

    return []
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(
      params,
      [
        'action', 'url', 'ref', 'value', 'key', 'direction', 'amount',
        'type', 'timeout', 'expression', 'method', 'params',
        'selector', 'interactiveOnly', 'compact', 'fullPage', 'clear',
      ],
      INPUT_PREVIEW_LINE_MAX,
    )
  },
}
