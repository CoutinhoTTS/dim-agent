import dns from 'node:dns/promises'
import http from 'node:http'
import https from 'node:https'
import net from 'node:net'
import tls from 'node:tls'
import { once } from 'node:events'
import { createBrotliDecompress, createGunzip, createInflate } from 'node:zlib'
import ipaddr from 'ipaddr.js'
import runtime from '../../package.json' with { type: 'json' }

export class WebError extends Error {
  code: string
  constructor(code: string, message: string) {
    super(message)
    this.code = code
  }
}
export type Diagnostic = { code: string; message: string }
export type NetworkConfig = { allowPrivateHosts: string[]; proxy?: URL }
export const DOWNLOAD_LIMIT = 5 * 1024 * 1024
type Address = ipaddr.IPv4 | ipaddr.IPv6
type Cidr = [Address, number]

export function hostname(url: URL): string {
  return url.hostname.replace(/^\[|\]$/g, '').toLowerCase().replace(/\.$/, '')
}

export function parseUrl(value: string): URL {
  if (/^https?:\/\/[^/?#]*@/i.test(value)) throw new WebError('invalid_input', 'User information is not allowed in URLs.')
  let url: URL
  try { url = new URL(value) } catch { throw new WebError('invalid_input', 'Expected an absolute HTTP or HTTPS URL.') }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new WebError('invalid_input', 'Expected an HTTP or HTTPS URL without user information.')
  }
  url.hash = ''
  return url
}

function matches(address: Address, [network, bits]: Cidr): boolean {
  if (address instanceof ipaddr.IPv4 && network instanceof ipaddr.IPv4) return address.match(network, bits)
  if (address instanceof ipaddr.IPv6 && network instanceof ipaddr.IPv6) return address.match(network, bits)
  return false
}

function effective(address: Address): Address {
  if (address instanceof ipaddr.IPv6) {
    if (address.isIPv4MappedAddress()) return address.toIPv4Address()
    const bytes = address.toByteArray()
    if (bytes.slice(0, 12).every(byte => byte === 0)) return new ipaddr.IPv4(bytes.slice(12))
  }
  return address
}

const RESERVED = ['198.18.0.0/15', '2001::/23', '3fff::/20'].map(value => ipaddr.parseCIDR(value))
function publicAddress(address: Address): boolean {
  const ip = effective(address)
  return ip.range() === 'unicast'
    && (!(ip instanceof ipaddr.IPv6) || (ip.toByteArray()[0] & 0xe0) === 0x20)
    && !RESERVED.some(cidr => matches(ip, cidr))
}

function policy(host: string, entries: string[], diagnostics: Diagnostic[]) {
  const hosts = new Set<string>()
  const ranges: Cidr[] = []
  let invalid = false
  for (const entry of entries) {
    try {
      if (entry.includes('/')) ranges.push(ipaddr.parseCIDR(entry))
      else {
        const host = domain(entry)
        if (!host) throw new Error('invalid host')
        hosts.add(host)
      }
    } catch { invalid = true }
  }
  if (invalid) diagnostics.push({ code: 'invalid_private_host', message: 'Ignored an invalid private-host exception.' })
  const allowedHost = hosts.has(host)
  if (!allowedHost && (host === 'localhost' || host.endsWith('.localhost'))) {
    throw new WebError('blocked_address', 'Localhost names are blocked.')
  }
  return (addresses: string[]) => {
    for (const value of addresses) {
      const address = ipaddr.parse(value)
      const permitted = allowedHost || ranges.some(cidr => matches(address, cidr) || matches(effective(address), cidr))
      if (!permitted && !publicAddress(address)) throw new WebError('blocked_address', 'Non-public addresses are blocked.')
    }
  }
}

export function domain(value: string): string | undefined {
  if (value.length > 253 || /[\s/@:#?%\[\]\\]/u.test(value)) return undefined
  let host: string
  try { host = hostname(new URL(`http://${value}`)) } catch { return undefined }
  return host.length <= 253 && host.split('.').every(label => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label)) ? host : undefined
}

export function proxyUrl(value?: string): URL | undefined {
  if (value === undefined) return undefined
  try {
    const proxy = parseUrl(value)
    if (proxy.pathname !== '/' || proxy.search) throw new Error('proxy origin required')
    return proxy
  } catch { throw new WebError('connect_failed', 'The configured proxy is unusable.') }
}

async function addresses(host: string, signal: AbortSignal): Promise<string[]> {
  signal.throwIfAborted()
  if (net.isIP(host)) return [host]
  return new Promise((resolve, reject) => {
    const aborted = () => reject(signal.reason)
    signal.addEventListener('abort', aborted, { once: true })
    void dns.lookup(host, { all: true, verbatim: true }).then(resolved => {
      signal.removeEventListener('abort', aborted)
      if (signal.aborted) reject(signal.reason)
      else if (resolved.length === 0) reject(new WebError('dns_failed', 'The host has no addresses.'))
      else resolve(resolved.map(item => item.address))
    }, () => {
      signal.removeEventListener('abort', aborted)
      reject(signal.aborted ? signal.reason : new WebError('dns_failed', 'The host could not be resolved.'))
    })
  })
}

async function tunnel(proxy: URL, ip: string, port: number, signal: AbortSignal): Promise<net.Socket> {
  const authority = `${net.isIP(ip) === 6 ? `[${ip}]` : ip}:${port}`
  const request = (proxy.protocol === 'https:' ? https : http).request(proxy, {
    method: 'CONNECT', path: authority, headers: { Host: authority }, agent: false, signal,
  })
  let transferred = false
  try {
    return await new Promise<net.Socket>((resolve, reject) => {
      request.once('error', reject)
      request.once('connect', (response, socket, head) => {
        if (response.statusCode !== 200) {
          socket.destroy()
          reject(new WebError('connect_failed', 'The proxy refused the tunnel.'))
        } else {
          if (head.length) socket.unshift(head)
          transferred = true
          resolve(socket)
        }
      })
      request.once('response', response => {
        response.destroy()
        reject(new WebError('connect_failed', 'The proxy refused the tunnel.'))
      })
      request.end()
    })
  } finally { if (!transferred) request.destroy() }
}

export type Response = {
  status: number; contentType: string; location?: string; body: Buffer; downloadLimited: boolean
}

/** One GET. Every redirect is a separate call, with a fresh address check. */
export async function get(url: URL, config: NetworkConfig, signal: AbortSignal, diagnostics: Diagnostic[], headers: Record<string, string> = {}): Promise<Response> {
  const proxy = config.proxy
  const host = hostname(url)
  const check = policy(host, config.allowPrivateHosts, diagnostics)
  const connecting = new AbortController()
  const connectTimer = setTimeout(() => connecting.abort(new WebError('timeout', 'Connection timed out.')), 10_000)
  const connectSignal = AbortSignal.any([signal, connecting.signal])
  let socket: net.Socket | undefined
  let response: http.IncomingMessage | undefined
  const abort = () => socket?.destroy()
  signal.addEventListener('abort', abort, { once: true })
  connectSignal.addEventListener('abort', abort, { once: true })
  try {
    const ips = await addresses(host, connectSignal)
    check(ips)
    connectSignal.throwIfAborted()
    const port = url.port ? Number(url.port) : url.protocol === 'https:' ? 443 : 80
    if (proxy) socket = await tunnel(proxy, ips[0], port, connectSignal)
    else {
      socket = net.connect({ host: ips[0], port })
      await once(socket, 'connect', { signal: connectSignal })
    }
    if (url.protocol === 'https:') {
      socket = tls.connect({
        socket, servername: net.isIP(host) ? undefined : host,
        checkServerIdentity: (_name, certificate) => tls.checkServerIdentity(host, certificate),
      })
      await once(socket, 'secureConnect', { signal: connectSignal })
    }
    clearTimeout(connectTimer)
    signal.throwIfAborted()
    const request = http.request({
      hostname: host, port, path: url.pathname + url.search, method: 'GET', signal,
      createConnection: () => socket!,
      headers: { Host: url.host, 'User-Agent': `dim-web/${runtime.version}`, Accept: 'text/html, text/plain, application/json, application/xml;q=0.9, */*;q=0.1', ...headers },
    })
    response = await new Promise<http.IncomingMessage>((resolve, reject) => {
      request.once('response', resolve)
      request.once('error', reject)
      request.end()
    })
    const status = response.statusCode!
    const contentType = response.headers['content-type'] ?? 'application/octet-stream'
    const result: Response = { status, contentType, location: response.headers.location, body: Buffer.alloc(0), downloadLimited: false }
    if (status < 200 || status >= 300) return result
    const encoding = response.headers['content-encoding']?.toLowerCase()
    const decoded = !encoding || encoding === 'identity' ? response
      : encoding === 'gzip' ? createGunzip()
        : encoding === 'deflate' ? createInflate()
          : encoding === 'br' ? createBrotliDecompress() : undefined
    if (!decoded) throw new WebError('connect_failed', 'The response encoding is unsupported.')
    if (!(decoded instanceof http.IncomingMessage)) {
      response.on('error', error => decoded.destroy(error))
      response.pipe(decoded)
    }
    const chunks: Buffer[] = []
    let size = 0
    try {
      for await (const part of decoded) {
        const chunk = Buffer.isBuffer(part) ? part : Buffer.from(part)
        const remaining = DOWNLOAD_LIMIT - size
        chunks.push(chunk.subarray(0, remaining))
        size += Math.min(chunk.length, remaining)
        if (chunk.length > remaining) { result.downloadLimited = true; break }
      }
    } finally { decoded.destroy() }
    result.body = Buffer.concat(chunks, size)
    return result
  } catch (error) {
    if (signal.aborted) throw signal.reason
    if (connecting.signal.aborted) throw connecting.signal.reason
    if (error instanceof WebError) throw error
    const code = (error as NodeJS.ErrnoException).code ?? ''
    if (/TLS|CERT|SELF_SIGNED|VERIFY/.test(code)) throw new WebError('tls_failed', 'TLS verification or negotiation failed.')
    throw new WebError('connect_failed', 'The connection failed.')
  } finally {
    clearTimeout(connectTimer)
    signal.removeEventListener('abort', abort)
    connectSignal.removeEventListener('abort', abort)
    response?.destroy()
    if (socket) {
      const closed = socket.closed ? Promise.resolve() : new Promise<void>(resolve => socket!.once('close', resolve))
      socket.destroy()
      await closed
    }
  }
}
