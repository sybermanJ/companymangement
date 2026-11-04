import { ref } from 'vue'
import { api, type Me } from './api'

export const me = ref<Me | null>(null)

export async function loadMe(): Promise<Me | null> {
  try {
    me.value = await api.get<Me>('/me')
  } catch {
    me.value = null
  }
  return me.value
}

export function currentMembership() {
  return me.value?.memberships.find((m) => m.tenant_id === me.value?.tenant_id)
}

const THEME_KEY = 'cm.theme'
export function applyTheme(theme?: 'light' | 'dark') {
  let t = theme
  if (!t) {
    try { t = (localStorage.getItem(THEME_KEY) as 'light' | 'dark' | null) ?? undefined } catch { /* storage blocked */ }
  }
  t ??= window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
  document.documentElement.dataset.theme = t
  if (theme) {
    try { localStorage.setItem(THEME_KEY, theme) } catch { /* ignore */ }
  }
}
