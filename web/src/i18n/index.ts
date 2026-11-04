import { createI18n } from 'vue-i18n'
import zhCN from './zh-CN'
import zhTW from './zh-TW'
import en from './en'
import ja from './ja'

export const LOCALES = [
  { code: 'zh-CN', label: '简体中文' },
  { code: 'zh-TW', label: '繁體中文' },
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
] as const
export type Locale = (typeof LOCALES)[number]['code']

const KEY = 'cm.locale'

function initial(): Locale {
  try {
    const saved = localStorage.getItem(KEY)
    if (saved && LOCALES.some((l) => l.code === saved)) return saved as Locale
  } catch { /* storage blocked */ }
  const nav = navigator.language
  if (nav.startsWith('zh')) return /TW|HK|Hant/i.test(nav) ? 'zh-TW' : 'zh-CN'
  if (nav.startsWith('ja')) return 'ja'
  if (nav.startsWith('en')) return 'en'
  return 'zh-CN'
}

export const i18n = createI18n({
  legacy: false,
  locale: initial(),
  fallbackLocale: 'zh-CN',
  messages: { 'zh-CN': zhCN, 'zh-TW': zhTW, en, ja },
})

export function setLocale(l: Locale) {
  i18n.global.locale.value = l
  document.documentElement.lang = l
  try { localStorage.setItem(KEY, l) } catch { /* ignore */ }
}
