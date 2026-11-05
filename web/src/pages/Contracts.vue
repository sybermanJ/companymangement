<script setup lang="ts">
// Contracts: staff see contracts of the projects they worked on; Finance, Admins and contract viewers see all.
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import Shell from './Shell.vue'
import ContractForm from './lc/ContractForm.vue'
import ContractPanel from './lc/ContractPanel.vue'
import { api, type Member } from '../lib/api'
import { errorText } from '../lib/errors'
import { money } from '../lib/money'
import { STATUS_TONE, type Contract } from '../lib/lifecycle'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const rows = ref<Contract[]>([])
const seesAll = ref(false)
const members = ref<Member[]>([])
const error = ref('')
const open = ref<string | null>(typeof route.query.id === 'string' ? route.query.id : null)
const creating = ref(route.query.new === '1')
const q = ref('')
const status = ref('')

async function load() {
  error.value = ''
  try {
    const d = await api.get<{ contracts: Contract[]; sees_all: boolean }>('/contracts')
    rows.value = d.contracts
    seesAll.value = d.sees_all
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(async () => {
  members.value = await api.get<Member[]>('/members').catch(() => [])
  await load()
})
watch(open, (v) => router.replace({ query: { id: v ?? undefined } }))
const shown = computed(() => {
  const s = q.value.trim().toLowerCase()
  return rows.value.filter((r) => (!status.value || r.status === status.value) && (!s || `${r.key} ${r.title} ${r.counterparty} ${r.project_key ?? ''} ${r.project_name ?? ''} ${r.owner_name}`.toLowerCase().includes(s)))
})
const totals = computed(() => {
  const m = new Map<string, [number, number]>()
  for (const r of shown.value) {
    const v = m.get(r.currency) ?? [0, 0]
    m.set(r.currency, [v[0] + r.amount_minor, v[1] + r.settled_minor])
  }
  return [...m.entries()].map(([c, [a, s]]) => t('lc.totalLine', { amount: money(a, c, locale.value), settled: money(s, c, locale.value) }))
})
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('nav.contracts') }}</h1>
        <button class="primary" type="button" @click="creating = true; open = null">+ {{ t('lc.newContract') }}</button>
      </div>
      <p class="muted">{{ seesAll ? t('lc.contractsAll') : t('lc.contractsMine') }}</p>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <ContractForm v-if="creating" :members="members" @saved="(id) => { creating = false; open = id; load() }" @cancel="creating = false" />
      <div class="layout" :class="{ withPanel: !!open }">
        <section class="card stack" style="min-width: 0">
          <div class="row">
            <input v-model="q" type="search" :placeholder="t('lc.searchContracts')" :aria-label="t('lc.searchContracts')" style="flex: 1 1 220px" />
            <select v-model="status" :aria-label="t('org.status')">
              <option value="">{{ t('lc.allStatuses') }}</option>
              <option v-for="s in ['draft', 'in_approval', 'returned', 'approved', 'signed', 'completed', 'terminated', 'rejected']" :key="s" :value="s">{{ t(`lc.s_${s}`) }}</option>
            </select>
          </div>
          <div v-if="totals.length" class="muted small">{{ totals.join(' · ') }}</div>
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('lc.contract') }}</th><th>{{ t('lc.counterparty') }}</th><th>{{ t('lc.project') }}</th><th class="num">{{ t('lc.contractAmount') }}</th><th>{{ t('org.status') }}</th></tr></thead>
              <tbody>
                <tr v-for="r in shown" :key="r.id" :class="{ sel: open === r.id }" tabindex="0" @click="open = r.id; creating = false" @keydown.enter="open = r.id">
                  <td><div class="muted small">{{ r.key }} · {{ t(`lc.ck_${r.kind}`) }} · {{ r.owner_name }}</div>{{ r.title }}</td>
                  <td>{{ r.counterparty }}</td>
                  <td>{{ r.project_key ?? '—' }}</td>
                  <td class="num">{{ money(r.amount_minor, r.currency, locale) }}<div v-if="r.settled_minor" class="muted small">{{ t('lc.settled') }} {{ money(r.settled_minor, r.currency, locale) }}</div></td>
                  <td><span class="pill" :class="STATUS_TONE[r.status]">{{ t(`lc.s_${r.status}`) }}</span></td>
                </tr>
                <tr v-if="!shown.length"><td colspan="5" class="empty">{{ t('lc.noContracts') }}</td></tr>
              </tbody>
            </table>
          </div>
        </section>
        <ContractPanel v-if="open" :id="open" :members="members" @close="open = null" @changed="load" />
      </div>
    </div>
  </Shell>
</template>

<style scoped>
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(340px, 500px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
.num { text-align: right; white-space: nowrap; }
tbody tr { cursor: pointer; }
tbody tr:hover, tbody tr.sel { background: var(--inset); }
</style>
