import type { TimelinePage, HistoryQuery, SubmitResult, InterruptResult, StateNotice, SubmitInput } from "./client-protocol.js"
import type { ResourceRef } from "./branch-services.js"
export type { ResourceRef } from "./branch-services.js"
export type { TimelinePage, HistoryQuery, SubmitResult, InterruptResult, StateNotice, SubmitInput, UserInput } from "./client-protocol.js"

export type Dispose = () => void
export interface PanelError extends Error {
  readonly name: "PanelError"
  readonly code: "binding_closed" | "invalid_binding" | "missing_capability"
}
export type PanelActivity = {
  readonly branch: Readonly<Pick<StateNotice, "running" | "stopping">>
  readonly sessionRunning: boolean
}
export interface PanelEnvironment {
  readonly sessionId: string
  readonly branchId: string
  readonly theme: "light" | "dark"
  readonly locale: string
}
export interface PanelContext<Services = PanelServices, Methods = Record<string, (input: Record<string, unknown>) => Promise<unknown>>> {
  readonly signal: AbortSignal
  readonly environment: PanelEnvironment
  observeEnvironment(listener: (value: PanelEnvironment) => void): Dispose
  readonly services: Services
  readonly methods: Methods
}
export interface PanelModule<Services = PanelServices, Methods = Record<string, (input: Record<string, unknown>) => Promise<unknown>>> {
  mount(root: ShadowRoot, context: PanelContext<Services, Methods>): Dispose
}
export interface PanelServices {
  session?: {
    state?: { observe(listener: (value: PanelActivity) => void): Dispose }
    history?: { read(query: HistoryQuery): Promise<TimelinePage> }
    submit?: (input: SubmitInput) => Promise<SubmitResult>
    interrupt?: () => Promise<InterruptResult>
  }
  resources?: { url(reference: ResourceRef): string }
}
export type PanelDeclaration = {
  id: string
  title: { default: string; translations?: Record<string, string> }
  icon?: string
  entry: string
  placement: string
  requires: string[]
  methods: Array<{ name: string; scope: "session" | "branch" }>
}
export type UiExtension = { pluginName: string; generation: string; panels: PanelDeclaration[] }
export type ExtensionDirectory = {
  revision: number
  extensions: UiExtension[]
  errors: Array<{ pluginName: string; message: string }>
}
