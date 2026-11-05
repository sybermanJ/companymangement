<script setup lang="ts">
// The project file: what it cost (by phase and category), its contracts and
// requests, and one timeline of what happened and why.
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Project, type ProjectCost } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { hours } from '../../lib/dates'
import { money, toMajor, toMinor } from '../../lib/money'
import { STATUS_TONE, type Claim, type Contract, type Expenses, type TimelineEntry } from '../../lib/lifecycle'

const props = defineProps<{ project: Project }>()
const { t, locale } = useI18n()
type File = { timeline: TimelineEntry[]; expenses: Expenses | null; cost: (ProjectCost & { expense_minor: number; total_minor: number; total_budget_minor: number | null }) | null; contracts: Contract[]; claims: Claim[]; can_set_budget: boolean }
const d = ref<File | null>(null)
const error = ref('')
const filter = ref<'all' | 'work' | 'money' | 'contract'>('all')
const ccy = computed(() => props.project.currency)
const m = (v: number | null | undefined) => money(v, ccy.value, locale.value)
const fmt = computed(() => new Intl.DateTimeFormat(locale.value, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))

async function load() {
  try {
    d.value = await api.get<File>(`/projects/${props.project.id}/file`)
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(load)

const shown = computed(() => (d.value?.timeline ?? []).filter((e) =>
  filter.value === 'all' || (filter.value === 'work' && (e.source === 'work' || e.source === 'log')) || (filter.value === 'money' && e.source === 'claim') || (filter.value === 'contract' && e.source === 'contract')))
const catMax = computed(() => Math.max(1, ...(d.value?.expenses?.by_category ?? []).map((x) => x[1])))
const pct = (a: number, b: number | null) => (b ? Math.round((a * 100) / b) : null)

function describe(e: TimelineEntry): string {
  if (e.source === 'log') return t('lc.tl_logged', { h: hours(e.minutes ?? 0) })
  if (e.source === 'work') {
    const p = (e.payload ?? {}) as Record<string, string>
    if (e.kind === 'status_changed') return t('work.ev_status_changed', { from: t(`work.s_${p.from}`), to: t(`work.s_${p.to}`) })
    if (e.kind === 'reassigned') return t('flow.ev_reassigned', { from: p.from_name ?? '', to: p.to_name ?? '' })
    return t(`lc.tl_${e.kind}`, { sha: String(p.sha ?? '').slice(0, 8), name: p.name ?? '' })
  }
  return t(`lc.a_${e.kind}`, { to: e.to ?? '' })
}
const link = (e: TimelineEntry) => (e.source === 'claim' ? `/requests?id=${e.id}` : e.source === 'contract' ? `/contracts?id=${e.id}` : null)

const editBudget = ref<string | null>(null)
async function saveBudget(path: string, v: string) {
  error.value = ''
  try {
    await api.put(path, { budget_minor: v.trim() === '' ? null : toMinor(v, ccy.value) })
    editBudget.value = null
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
const budgetInput = ref('')
function startEdit(key: string, current: number | null) {
  editBudget.value = key
  budgetInput.value = current === null ? '' : String(toMajor(current, ccy.value))
}
</script>

<template>
  <div class="stack">
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <template v-if="d">
      <div class="row" style="justify-content: space-between">
        <p class="muted" style="margin: 0">{{ t('lc.fileIntro') }}</p>
        <div class="row">
          <RouterLink :to="`/requests?new=reimbursement&project=${project.id}`" class="btn">+ {{ t('lc.newRequest') }}</RouterLink>
          <RouterLink :to="`/requests?new=trip&project=${project.id}`" class="btn">+ {{ t('lc.newTrip') }}</RouterLink>
        </div>
      </div>
      <div v-if="d.expenses" class="grid-auto">
        <div v-if="d.cost" class="card tile"><div class="muted small">{{ t('fin.spent') }}</div><div class="big">{{ m(d.cost.spent_minor) }}</div><div class="muted small">{{ t('work.budget') }} {{ m(d.cost.budget_minor) }}</div></div>
        <div class="card tile">
          <div class="muted small">{{ t('lc.expenses') }}</div><div class="big">{{ m(d.expenses.total_minor) }}</div>
          <div class="muted small">
            {{ t('lc.expenseBudget') }}
            <template v-if="editBudget === 'project'">
              <form class="inline" @submit.prevent="saveBudget(`/projects/${project.id}/expense-budget`, budgetInput)"><input v-model="budgetInput" type="number" min="0" step="0.01" :aria-label="t('lc.expenseBudget')" /><button type="submit">{{ t('common.save') }}</button></form>
            </template>
            <template v-else>{{ m(d.expenses.budget_minor) }}<template v-if="pct(d.expenses.total_minor, d.expenses.budget_minor) !== null"> · {{ pct(d.expenses.total_minor, d.expenses.budget_minor) }}%</template>
              <button v-if="d.can_set_budget" type="button" class="link" @click="startEdit('project', d.expenses.budget_minor)">{{ t('lc.set') }}</button></template>
          </div>
        </div>
        <div v-if="d.cost" class="card tile"><div class="muted small">{{ t('lc.totalCost') }}</div><div class="big">{{ m(d.cost.total_minor) }}</div><div class="muted small">{{ t('work.budget') }} {{ m(d.cost.total_budget_minor) }}</div></div>
        <div class="card tile"><div class="muted small">{{ t('lc.inApproval') }}</div><div class="big">{{ m(d.expenses.in_approval_minor) }}</div><div class="muted small">{{ t('lc.advancesOut') }} {{ m(d.expenses.advances_outstanding_minor) }}</div></div>
      </div>
      <p v-if="d.expenses?.pending_fx" class="notice warn">{{ t('lc.pendingFx', { n: d.expenses.pending_fx }) }}</p>

      <section v-if="d.expenses" class="card stack">
        <h2>{{ t('lc.byPhase') }}</h2>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('lc.phase') }}</th><th>{{ t('lc.step_phase_manager') }}</th><th class="num">{{ t('work.logged') }}</th><th class="num">{{ t('lc.expenses') }}</th><th class="num">{{ t('lc.phaseBudget') }}</th></tr></thead>
            <tbody>
              <tr v-for="p in d.expenses.by_phase" :key="p.phase_id">
                <td>{{ p.title }}</td><td>{{ p.manager ?? t('lc.pmFallback') }}</td><td class="num">{{ hours(p.logged_minutes) }} {{ t('work.hours') }}</td>
                <td class="num">{{ m(p.spent_minor) }}<div v-if="pct(p.spent_minor, p.budget_minor) !== null" class="bar" :aria-label="`${pct(p.spent_minor, p.budget_minor)}%`"><i :style="{ width: Math.min(100, pct(p.spent_minor, p.budget_minor)!) + '%' }" :class="{ over: pct(p.spent_minor, p.budget_minor)! > 100 }" /></div></td>
                <td class="num">
                  <form v-if="editBudget === p.phase_id" class="inline" @submit.prevent="saveBudget(`/phases/${p.phase_id}/budget`, budgetInput)"><input v-model="budgetInput" type="number" min="0" step="0.01" :aria-label="t('lc.phaseBudget')" /><button type="submit">{{ t('common.save') }}</button></form>
                  <template v-else>{{ m(p.budget_minor) }} <button v-if="d.can_set_budget" type="button" class="link" @click="startEdit(p.phase_id, p.budget_minor)">{{ t('lc.set') }}</button></template>
                </td>
              </tr>
              <tr v-if="!d.expenses.by_phase.length"><td colspan="5" class="empty">{{ t('lc.noPhases') }}</td></tr>
            </tbody>
          </table>
        </div>
        <h2>{{ t('lc.byCategory') }}</h2>
        <ul class="cats">
          <li v-for="[k, v] in d.expenses.by_category" :key="k"><span>{{ t(`lc.c_${k}`) }}</span><span class="bar wide"><i :style="{ width: (v * 100) / catMax + '%' }" /></span><span class="num">{{ m(v) }}</span></li>
          <li v-if="!d.expenses.by_category.length" class="empty">{{ t('lc.noExpenses') }}</li>
        </ul>
      </section>

      <div class="two">
        <section class="card stack">
          <h2>{{ t('nav.contracts') }}</h2>
          <ul class="list">
            <li v-for="c in d.contracts" :key="c.id"><RouterLink :to="`/contracts?id=${c.id}`">{{ c.key }} {{ c.title }}</RouterLink> <span class="pill" :class="STATUS_TONE[c.status]">{{ t(`lc.s_${c.status}`) }}</span><div class="muted small">{{ c.counterparty }} · {{ money(c.amount_minor, c.currency, locale) }}<template v-if="c.settled_minor"> · {{ t('lc.settled') }} {{ money(c.settled_minor, c.currency, locale) }}</template></div></li>
            <li v-if="!d.contracts.length" class="empty">{{ t('lc.noContracts') }}</li>
          </ul>
          <RouterLink to="/contracts?new=1" class="small">+ {{ t('lc.newContract') }}</RouterLink>
        </section>
        <section class="card stack">
          <h2>{{ t('nav.requests') }}</h2>
          <ul class="list">
            <li v-for="c in d.claims.slice(0, 12)" :key="c.id"><RouterLink :to="`/requests?id=${c.id}`">{{ c.key }} {{ c.title }}</RouterLink> <span class="pill" :class="STATUS_TONE[c.status]">{{ t(`lc.s_${c.status}`) }}</span><div class="muted small">{{ c.applicant_name }}<template v-if="c.phase_title"> · {{ c.phase_title }}</template> · {{ money(c.amount_minor, c.currency, locale) }}</div></li>
            <li v-if="!d.claims.length" class="empty">{{ t('lc.empty_mine') }}</li>
          </ul>
        </section>
      </div>

      <section class="card stack">
        <div class="row" style="justify-content: space-between">
          <h2>{{ t('lc.timeline') }}</h2>
          <div role="group" class="seg" :aria-label="t('lc.timeline')">
            <button v-for="k in ['all', 'work', 'money', 'contract'] as const" :key="k" type="button" :class="{ primary: filter === k }" :aria-pressed="filter === k" @click="filter = k">{{ t(`lc.tf_${k}`) }}</button>
          </div>
        </div>
        <ol class="tl">
          <li v-for="(e, i) in shown" :key="i" :class="e.source">
            <span class="muted small when">{{ fmt.format(new Date(e.at)) }}</span>
            <div>
              <div><strong>{{ e.who }}</strong> {{ describe(e) }}
                <component :is="link(e) ? 'RouterLink' : 'span'" :to="link(e) ?? undefined" class="ref">{{ e.ref }} {{ e.title }}</component>
                <span v-if="e.amount_minor !== undefined && e.kind === 'submit'" class="muted"> · {{ money(e.amount_minor, e.currency ?? ccy, locale) }}</span>
              </div>
              <div v-if="e.note" class="muted small">“{{ e.note }}”</div>
              <div v-if="e.kind === 'submit' && e.justification" class="muted small">{{ t('lc.why') }}：{{ e.justification }}</div>
            </div>
          </li>
          <li v-if="!shown.length" class="empty">{{ t('lc.noEvents') }}</li>
        </ol>
      </section>
    </template>
  </div>
</template>

<style scoped>
.tile { padding: 14px 16px; }
.big { font-size: 22px; font-weight: 700; margin-top: 4px; }
.num { text-align: right; white-space: nowrap; }
.bar { display: block; height: 6px; border-radius: 3px; background: var(--inset); margin-top: 4px; overflow: hidden; min-width: 60px; }
.bar i { display: block; height: 100%; background: var(--accent); }
.bar i.over { background: var(--danger); }
.bar.wide { flex: 1 1 auto; margin: 0; }
.cats { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.cats li { display: grid; grid-template-columns: 110px 1fr 130px; gap: 10px; align-items: center; }
.two { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 16px; }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.seg { display: flex; flex-wrap: wrap; }
.seg button { border-radius: 0; margin-left: -1px; }
.seg button:first-child { border-radius: 8px 0 0 8px; margin-left: 0; }
.seg button:last-child { border-radius: 0 8px 8px 0; }
.tl { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
.tl li { display: grid; grid-template-columns: 150px 1fr; gap: 10px; padding: 8px 0 8px 12px; border-left: 3px solid var(--line); }
.tl li.claim { border-left-color: var(--warn); }
.tl li.contract { border-left-color: var(--info); }
.tl li.work { border-left-color: var(--accent); }
.ref { margin-left: 4px; }
.inline { display: inline-flex; gap: 4px; }
.inline input { height: 30px; width: 110px; }
.inline button { height: 30px; padding: 0 8px; }
button.link { height: auto; padding: 0 4px; border: 0; background: none; color: var(--accent); text-decoration: underline; }
.btn { display: inline-flex; align-items: center; height: 40px; padding: 0 16px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); text-decoration: none; }
@media (max-width: 640px) { .tl li { grid-template-columns: 1fr; gap: 0; } .cats li { grid-template-columns: 90px 1fr 110px; } }
</style>
