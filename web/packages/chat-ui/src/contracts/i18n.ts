import type { Ref } from 'vue'
import type { ChatUiMessageKey } from '../i18n'

export interface ChatUiI18n {
  readonly locale: Readonly<Ref<string>>
  t(key: ChatUiMessageKey, params?: Record<string, string | number>): string
}
