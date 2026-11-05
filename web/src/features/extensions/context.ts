import type { Dispose, PanelActivity, PanelContext, PanelDeclaration, PanelEnvironment, PanelError as PublicPanelError, PanelServices } from "../../../../npm/ui"
import { RpcError, type RpcClient } from "../../rpc/client"
import { readInterrupt, readSubmit, readTimelinePage } from "../../rpc/read"

export const capabilities = ["session.state", "session.history", "session.submit", "session.interrupt", "resources"]
export function panelUnavailable(panel: PanelDeclaration, supports: (method: string) => boolean, methods: readonly string[]): string | undefined {
  if (panel.placement !== "session-panel") return `Unsupported placement: ${panel.placement}`
  for (const capability of panel.requires) {
    if (!capabilities.includes(capability)) return `Unsupported capability: ${capability}`
    const method = { "session.history": "session.history", "session.submit": "session.submit", "session.interrupt": "session.stop" }[capability]
    if (method && !supports(method)) return `Unavailable capability: ${capability}`
  }
  for (const method of panel.methods) if (!methods.includes(method.name)) return `Unavailable method: ${method.name}`
}
export class PanelError extends Error implements PublicPanelError {
  override readonly name = "PanelError"
  constructor(readonly code: PublicPanelError["code"], message: string) { super(message) }
}

type Host = {
  client: Pick<RpcClient, "call">
  environment: PanelEnvironment
  observeEnvironment(listener: (value: PanelEnvironment) => void): Dispose
  observeActivity(listener: (value: PanelActivity) => void): Dispose
  unavailable(error: Error): void
}

export function createPanelContext(panel: PanelDeclaration, host: Host): { context: PanelContext; dispose: Dispose } {
  const controller = new AbortController()
  const cleanups = new Set<Dispose>()
  const { sessionId, branchId } = host.environment
  const bound = Object.freeze({ sessionId, branchId })
  function active(): void {
    if (controller.signal.aborted) throw new PanelError("binding_closed", "This panel binding has ended")
  }
  function observe<T>(subscribe: (listener: (value: T) => void) => Dispose, listener: (value: T) => void): Dispose {
    active()
    const stop = subscribe(value => { if (!controller.signal.aborted) listener(value) })
    cleanups.add(stop)
    return () => { cleanups.delete(stop); stop() }
  }
  async function call<T>(method: string, input: Record<string, unknown>, read: (value: unknown) => T, branch: boolean): Promise<T> {
    active()
    if ("sessionId" in input || "branchId" in input) throw new PanelError("invalid_binding", "Panel arguments cannot override their bound target")
    try {
      const result = await host.client.call(method, { ...input, sessionId, ...(branch ? { branchId } : {}) }, read)
      active()
      return result
    } catch (error) {
      if (!controller.signal.aborted && error instanceof RpcError && ["service_reloading", "service_unavailable", "unknown_method", "session_closed", "branch_ended"].includes(error.appError ?? "")) host.unavailable(error)
      throw error
    }
  }
  const services: PanelServices = {}
  for (const requirement of panel.requires) {
    switch (requirement) {
      case "session.state":
        (services.session ??= {}).state = { observe: listener => observe(host.observeActivity, listener) }
        break
      case "session.history":
        (services.session ??= {}).history = { read: query => call("session.history", query, readTimelinePage, true) }
        break
      case "session.submit":
        (services.session ??= {}).submit = input => call("session.submit", input, readSubmit, true)
        break
      case "session.interrupt":
        (services.session ??= {}).interrupt = () => call("session.stop", {}, readInterrupt, true)
        break
      case "resources":
        services.resources = { url(reference) {
          active()
          if (reference.sessionId !== bound.sessionId || !/^[a-zA-Z0-9_-]+$/.test(reference.id)) throw new PanelError("invalid_binding", "Resource must belong to this Session")
          return `/api/ui/sessions/${encodeURIComponent(bound.sessionId)}/resources/${encodeURIComponent(reference.id)}`
        } }
        break
      default: throw new PanelError("missing_capability", `Unsupported capability: ${requirement}`)
    }
  }
  const methods = Object.fromEntries(panel.methods.map(method => [method.name, (input: Record<string, unknown>) => call(method.name, input, value => value, method.scope === "branch")]))
  const context: PanelContext = Object.freeze({
    signal: controller.signal,
    environment: Object.freeze({ ...host.environment }),
    observeEnvironment: (listener: (value: PanelEnvironment) => void) => observe(host.observeEnvironment, listener),
    services: Object.freeze(services),
    methods: Object.freeze(methods),
  })
  return { context, dispose() {
    controller.abort()
    for (const cleanup of cleanups) {
      try { cleanup() } catch (error) { console.error("Extension observer cleanup failed", error) }
    }
    cleanups.clear()
  } }
}

export function resourceUrl(generation: string, path: string): string {
  return `/api/ui/extensions/${encodeURIComponent(generation)}/${path.split("/").map(encodeURIComponent).join("/")}`
}
export function panelTitle(panel: PanelDeclaration, locale: string): string {
  return panel.title.translations?.[locale] ?? panel.title.translations?.[locale.split("-")[0]!] ?? panel.title.default
}
