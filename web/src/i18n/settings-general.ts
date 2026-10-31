import { useI18n } from "vue-i18n"

export function useSettingsText(): (english: string, chinese: string) => string {
  const { locale } = useI18n()
  return (english, chinese) => locale.value.startsWith("zh") ? chinese : english
}
