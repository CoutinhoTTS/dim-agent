import { inject, type InjectionKey, type Ref } from 'vue'
import type { ChatUiCapabilities } from './capabilities'
import type { ChatUiCommands } from './commands'
import type { ChatUiI18n } from './i18n'
import type { ChatUiSessionView } from './session'
import type { ChatUiTimelineHost } from './timeline'

export interface ChatUiHost {
  readonly session: Readonly<Ref<ChatUiSessionView | null>>
  readonly capabilities: Readonly<Ref<ChatUiCapabilities>>
  readonly commands: ChatUiCommands
  readonly i18n: ChatUiI18n
  readonly timeline?: ChatUiTimelineHost
}

export const CHAT_UI_HOST: InjectionKey<ChatUiHost> = Symbol('chat-ui-host')

export function useChatUiHost(): ChatUiHost {
  const host = useOptionalChatUiHost()
  if (host === undefined) {
    throw new Error('[chat-ui] Missing ChatUiHost provider. Provide CHAT_UI_HOST above the shared chat UI.')
  }
  return host
}

export function useOptionalChatUiHost(): ChatUiHost | undefined {
  return inject(CHAT_UI_HOST, undefined)
}
