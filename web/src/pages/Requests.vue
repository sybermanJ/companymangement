<script setup lang="ts">
// Expense and fund requests: mine, waiting for my approval, waiting for payment, all.
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import Shell from './Shell.vue'
import RequestForm from './lc/RequestForm.vue'
import RequestPanel from './lc/RequestPanel.vue'
import { api, type Member } from '../lib/api'
import { errorText } from '../lib/errors'
import { money } from '../lib/money'
import { currentMembership } from '../lib/session'
import { STATUS_TONE, type Claim } from '../lib/lifecycle'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const sees = computed(() => !!(currentMembership()?.is_admin || currentMembership()?.is_finance))
type View = 'mine' | 'to_approve' | 'to_pay' | 'all'
const views = computed<View[]>(() => (sees.value ? ['mine', 'to_approve', 'to_pay', 'all'] : ['mine', 'to_approve']))
const view = ref<View>((route.query.view as View) || 'mine')
const rows = ref<Claim[]>([])
const members = ref<Member[]>([])
const error = ref('')
const open = ref<string | null>(typeof route.query.id === 'string' ? route.query.id : null)
const creating = ref<null | { kind?: string; parent?: string; project?: string; phase?: string }>(route.query.new ? { kind: String(route.query.new), project: route.query.project as string | undefined } : null)
const q = ref('')

async function load() {
  error.value = ''
  try {
    rows.value = await api.get<Claim[]>(`/claims?view=${view.value}`)
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(async () => {
  members.value = await api.get<Member[]>('/members').catch(() => [])
  await load()
})
watch(view, () => { router.replace({ query: { ...route.query, view: view.value } }); load() })
watch(open, (v) => router.replace({ query: { ...route.query, id: v ?? undefined } }))
watch(() => route.query.id, (v) => { if (typeof v === 'string') open.value = v })

const shown = computed(() => {
  const s = q.value.trim().toLowerCase()
  return s ? rows.value.filter((r) => `${r.key} ${r.title} ${r.applicant_name} ${r.project_key ?? ''} ${r.project_name ?? ''} ${r.payee}`.toLowerCase().includes(s)) : rows.value
})
const totals = computed(() => {
  const m = new Map<string, number>()
  for (const r of shown.value) if (r.kind !== 'trip') m.set(r.currency, (m.get(r.currency) ?? 0) + r.amount_minor)
  return [...m.entries()].map(([c, v]) => money(v, c, locale.value)).join(' · ')
})
function saved(id: string) {
  creating.value = null
  open.value = id
  view.value = 'mine'
  load()
}
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('nav.requests') }}</h1>
        <div class="row">
          <button type="button" @click="creating = { kind: 'trip' }; open = null">+ {{ t('lc.newTrip') }}</button>
          <button class="primary" type="button" @click="creating = { kind: 'reimbursement' }; open = null">+ {{ t('lc.newRequest') }}</button>
        </div>
      </div>
      <p class="muted">{{ t('lc.requestsIntro') }}</p>
      <div role="tablist" class="tabs" :aria-label="t('nav.requests')">
        <button v-for="k in views" :key="k" role="tab" type="button" :aria-selected="view === k" :class="{ on: view === k }" @click="view = k">{{ t(`lc.v_${k}`) }}</button>
      </div>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <RequestForm v-if="creating" :key="JSON.stringify(creating)" :members="members" :initial="creating" @saved="saved" @cancel="creating = null" />
      <div class="layout" :class="{ withPanel: !!open }">
        <section class="card stack" style="min-width: 0">
          <div class="row" style="justify-content: space-between">
            <input v-model="q" type="search" :placeholder="t('lc.search')" :aria-label="t('lc.search')" style="flex: 1 1 220px" />
            <span v-if="shown.length" class="muted small">{{ t('lc.count', { n: shown.length }) }}<template v-if="totals"> · {{ totals }}</template></span>
          </div>
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('lc.request') }}</th><th>{{ t('lc.project') }}</th><th>{{ t('lc.applicant') }}</th><th class="num">{{ t('lc.amount') }}</th><th>{{ t('org.status') }}</th></tr></thead>
              <tbody>
                <tr v-for="r in shown" :key="r.id" :class="{ sel: open === r.id }" tabindex="0" @click="open = r.id; creating = null" @keydown.enter="open = r.id">
                  <td><div class="muted small">{{ r.key }} · {{ t(`lc.k_${r.kind}`) }}</div>{{ r.title }}</td>
                  <td>{{ r.project_key ?? '—' }}<div v-if="r.phase_title" class="muted small">{{ r.phase_title }}</div></td>
                  <td>{{ r.applicant_name }}</td>
                  <td class="num">{{ money(r.amount_minor, r.currency, locale) }}</td>
                  <td><span class="pill" :class="STATUS_TONE[r.status]">{{ t(`lc.s_${r.status}`) }}</span></td>
                </tr>
                <tr v-if="!shown.length"><td colspan="5" class="empty">{{ t(`lc.empty_${view}`) }}</td></tr>
              </tbody>
            </table>
          </div>
        </section>
        <RequestPanel v-if="open" :id="open" :members="members" @close="open = null" @changed="load" @open="(id) => (open = id)" @file-for="(trip) => { creating = { kind: 'reimbursement', parent: trip.id, project: trip.project_id ?? undefined, phase: trip.phase_id ?? undefined }; open = null }" />
      </div>
    </div>
  </Shell>
</template>

<style scoped>
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(340px, 480px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
.num { text-align: right; white-space: nowrap; }
tbody tr { cursor: pointer; }
tbody tr:hover, tbody tr.sel { background: var(--inset); }
</style>
