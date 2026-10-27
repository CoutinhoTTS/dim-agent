import { parse } from 'node-html-parser'
import { NodeHtmlMarkdown } from 'node-html-markdown'
import { MIMEType } from 'node:util'
import { WebError, type Diagnostic } from './network.ts'

declare global {
  interface JSON {
    parse(text: string, reviver: (key: string, value: unknown, context: { source?: string }) => unknown): unknown
    rawJSON(text: string): unknown
  }
}

export function plainText(html: string): string {
  return parse(html).textContent.replace(/\s+/gu, ' ').trim()
}

export function redact(text: string, key?: string): string {
  if (!key) return text
  return text.replaceAll(key, '[redacted]').replaceAll(encodeURIComponent(key), '[redacted]')
}

export function convert(body: Buffer, contentType: string, url: URL, diagnostics: Diagnostic[]): { text: string; title?: string } {
  let mime: MIMEType
  try { mime = new MIMEType(contentType) }
  catch { throw new WebError('unsupported_content_type', 'The response has an invalid content type.') }
  const media = mime.essence
  const html = media === 'text/html' || media === 'application/xhtml+xml'
  if (!html && !['text/plain', 'text/markdown', 'text/csv', 'application/json', 'application/xml', 'text/xml'].includes(media)
      && !media.endsWith('+json') && !media.endsWith('+xml')) {
    throw new WebError('unsupported_content_type', `Unsupported content type: ${media}.`)
  }
  const headerCharset = mime.params.get('charset')
  const metaCharset = html ? /<meta\b[^>]*\bcharset\s*=\s*["']?([^\s;"'/>]+)/i.exec(body.subarray(0, 1024).toString('latin1'))?.[1] : undefined
  let decoder: TextDecoder
  try { decoder = new TextDecoder(headerCharset ?? metaCharset ?? 'utf-8') }
  catch {
    decoder = new TextDecoder('utf-8')
    diagnostics.push({ code: 'unsupported_charset', message: 'An unsupported charset was decoded as UTF-8 with replacement.' })
  }
  const decoded = decoder.decode(body)
  if (!html) {
    if (media === 'application/json' || media.endsWith('+json')) {
      try {
        const value = JSON.parse(decoded, (_key, item, context) => typeof item === 'number' ? JSON.rawJSON(context.source!) : item)
        return { text: JSON.stringify(value, null, 2) }
 } catch { }
    }
    return { text: decoded }
  }
  const document = parse(decoded)
  const title = document.querySelector('title')?.textContent.replace(/\s+/gu, ' ').trim()
  for (const element of document.querySelectorAll('script,style,noscript,template,svg,iframe,nav,input,button,select,textarea,[role="navigation"]')) element.remove()
  let base = url
  const declaredBase = document.querySelector('base[href]')?.getAttribute('href')
  if (declaredBase !== undefined) {
    try {
      const target = new URL(declaredBase, url)
      if (['http:', 'https:'].includes(target.protocol) && !target.username && !target.password) base = target
 } catch { }
  }
  for (const anchor of document.querySelectorAll('a[href]')) {
    try {
      const target = new URL(anchor.getAttribute('href')!, base)
      if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password) anchor.removeAttribute('href')
      else anchor.setAttribute('href', target.href)
    } catch { anchor.removeAttribute('href') }
  }
  const main = document.querySelector('main') ?? document.querySelector('article') ?? document.querySelector('body') ?? document
  return { title, text: NodeHtmlMarkdown.translate(main.toString(), { ignore: ['IMG'], maxConsecutiveNewlines: 2 }).trim() }
}
