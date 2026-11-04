<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { LOCALES, setLocale, type Locale } from '../i18n'
import { api } from '../lib/api'
import { me, applyTheme, currentMembership } from '../lib/session'
import { unread, startLive, stopLive } from '../lib/live'
import { computed, onMounted, ref } from 'vue'

const { t, locale } = useI18n()
const router = useRouter()
const theme = ref(document.documentElement.dataset.theme ?? 'light')
const sees = computed(() => !!(currentMembership()?.is_admin || currentMembership()?.is_finance))
const NAV = computed(() => [
  { k: 'overview', to: '/overview' }, { k: 'projects', to: '/projects' }, { k: 'team', to: '/team' },
  { k: 'mine', to: '/my' }, { k: 'routines', to: '/routines' },
  ...(sees.value ? [{ k: 'finance', to: '/finance' }] : []), { k: 'org', to: '/org' },
])
onMounted(startLive)

function toggleTheme() {
  theme.value = theme.value === 'dark' ? 'light' : 'dark'
  applyTheme(theme.value as 'light' | 'dark')
}
async function logout() {
  await api.post('/logout').catch(() => undefined)
  stopLive()
  me.value = null
  router.push('/login')
}
async function resend() {
  await api.post('/verify-email/resend')
  resent.value = true
}
const resent = ref(false)
</script>

<template>
  <div>
    <header class="topbar">
      <span class="brand">{{ t('brand') }}</span>
      <nav class="nav" :aria-label="t('nav.org')">
        <RouterLink v-for="n in NAV" :key="n.k" :to="n.to" class="nav-item" active-class="active">{{ t(`nav.${n.k}`) }}</RouterLink>
      </nav>
      <span class="spacer" />
      <RouterLink to="/inbox" class="nav-item inbox" active-class="active" :aria-label="unread ? t('nav.inboxUnread', { n: unread }) : t('nav.inbox')">
        {{ t('nav.inbox') }}<span v-if="unread" class="count" aria-hidden="true">{{ unread > 99 ? '99+' : unread }}</span>
      </RouterLink>
      <label class="sr-only" for="lang">{{ t('common.language') }}</label>
      <select id="lang" :value="locale" @change="setLocale(($event.target as HTMLSelectElement).value as Locale)">
        <option v-for="l in LOCALES" :key="l.code" :value="l.code">{{ l.label }}</option>
      </select>
      <button type="button" @click="toggleTheme">{{ theme === 'dark' ? t('common.light') : t('common.dark') }}</button>
      <span v-if="me" class="who">{{ me.name }}</span>
      <button type="button" @click="logout">{{ t('common.logout') }}</button>
    </header>
    <div v-if="me && !me.email_verified" class="wrap">
      <div class="notice warn row">
        <span>{{ t('auth.unverified') }}</span>
        <button v-if="!resent" type="button" @click="resend">{{ t('auth.resend') }}</button>
        <span v-else>{{ t('auth.resent') }}</span>
      </div>
    </div>
    <main class="wrap">
      <slot />
    </main>
  </div>
</template>

<style scoped>
.topbar { display: flex; flex-wrap: wrap; align-items: center; gap: 12px 16px; padding: 10px 24px; background: var(--surface); border-bottom: 1px solid var(--line); }
.nav { display: flex; flex-wrap: wrap; gap: 4px; }
.nav-item { padding: 8px 12px; border-radius: 8px; color: var(--text-2); text-decoration: none; }
.nav-item.active { background: var(--accent-soft); color: var(--accent); font-weight: 500; }
.inbox { position: relative; display: inline-flex; align-items: center; gap: 6px; }
.count { min-width: 20px; height: 20px; padding: 0 6px; border-radius: 10px; background: var(--danger); color: var(--surface); font-size: 12px; font-weight: 600; display: inline-flex; align-items: center; justify-content: center; }
.spacer { flex: 1 1 auto; }
.who { color: var(--text-2); }
.wrap { max-width: 1240px; margin: 0 auto; padding: 20px 24px 0; }
main.wrap { padding-bottom: 40px; }
.sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
@media (max-width: 640px) { .topbar, .wrap { padding-left: 16px; padding-right: 16px; } }
</style>
