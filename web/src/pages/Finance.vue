<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import Shell from './Shell.vue'
import { api, type Member, type Project, type ProjectCost } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'
import { money, toMinor, toMajor } from '../lib/money'

interface Alert { id: string; project_id: string; project_key: string; project_name: string; currency: string; metric: string; mode: string; percent: number | null; level: string; observed_minor: number; limit_minor: number; state: string; fired_at: string; ack_by_name: string | null; ack_note: string }
interface Threshold { id: string; metric: string; mode: string; amount_minor: number | null; percent: number | null; level: string; notify_pm: boolean; notify_finance: boolean; notify_admins: boolean; alert_state: string | null; alert_fired_at: string | null }
interface Person { user_id: string; user_name: string; minutes: number; cost_minor: number; rate_minor: number | null; rate_currency: string | null }
interface Rate { id: string; user_id: string; user_name: string; rate_minor: number; currency: string; effective_from: string; note: string; set_by_name: string }
interface Fx { id: string; from_ccy: string; to_ccy: string; rate: string; effective_date: string; set_by_name: string }
interface Period { id: string; starts_on: string; ends_on: string; status: string; closed_by_name: string | null; closed_at: string | null; entries: number }
interface Audit { id: string; actor_name: string; action: string; payload: Record<string, unknown>; created_at: string }

const { t, locale } = useI18n()
const route = useRoute()
type Tab = 'costs' | 'rates' | 'fx' | 'periods' | 'audit'
const TABS: Tab[] = ['costs', 'rates', 'fx', 'periods', 'audit']
const tab = ref<Tab>('costs')
const error = ref('')
const notice = ref('')

const overview = ref<{ projects: { project: Project; cost: ProjectCost }[]; alerts: Alert[]; pending_fx: { from: string; to: string; earliest: string; entries: number }[]; missing_rates: { user_name: string; entries: number; earliest: string }[] } | null>(null)
const projectId = ref<string>(typeof route.query.project === 'string' ? route.query.project : '')
const detail = ref<{ project: Project; cost: ProjectCost; people: Person[]; thresholds: Threshold[]; alerts: Alert[] } | null>(null)
const members = ref<Member[]>([])
const rates = ref<Rate[]>([])
const fx = ref<{ rates: Fx[]; pending: { from: string; to: string; earliest: string; entries: number }[] }>({ rates: [], pending: [] })
const periods = ref<Period[]>([])
const audit = ref<Audit[]>([])
const fmtDate = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))
const m = (minor: number | null | undefined, ccy: string) => money(minor, ccy, locale.value)

async function loadOverview() {
  overview.value = await api.get('/finance/overview')
  if (!projectId.value && overview.value?.projects.length) projectId.value = overview.value.projects[0].project.id
}
async function loadDetail() {
  if (!projectId.value) return
  detail.value = await api.get(`/finance/projects/${projectId.value}`)
  budget.value = detail.value?.cost.budget_minor != null ? String(toMajor(detail.value.cost.budget_minor, detail.value.cost.currency)) : ''
}
async function loadTab() {
  if (tab.value === 'rates') {
    rates.value = await api.get('/finance/rates')
    if (!members.value.length) members.value = await api.get('/members')
  } else if (tab.value === 'fx') fx.value = await api.get('/finance/fx')
  else if (tab.value === 'periods') periods.value = await api.get('/finance/periods')
  else if (tab.value === 'audit') audit.value = await api.get('/finance/audit')
}
async function reload() {
  try {
    await loadOverview()
    await loadDetail()
    await loadTab()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(reload)
watch(projectId, () => loadDetail().catch((e) => (error.value = errorText(e, t))))
watch(tab, () => loadTab().catch((e) => (error.value = errorText(e, t))))

async function run(fn: () => Promise<unknown>, ok?: string) {
  error.value = ''
  notice.value = ''
  try {
    const r = await fn()
    if (ok) notice.value = typeof r === 'string' ? r : ok
    await reload()
  } catch (e) {
    error.value = errorText(e, t)
  }
}

// Cost bar with threshold markers.
const scaleMax = computed(() => {
  const d = detail.value
  if (!d) return 1
  const limits = d.thresholds.map((x) => limitOf(x) ?? 0)
  return Math.max(d.cost.budget_minor ?? 0, d.cost.spent_minor, d.cost.forecast_minor, ...limits, 1)
})
function limitOf(x: Threshold): number | null {
  if (x.mode === 'absolute') return x.amount_minor
  const b = detail.value?.cost.budget_minor
  return b && x.percent ? Math.ceil((b * x.percent) / 100) : null
}
const pct = (v: number) => `${Math.min(100, (v / scaleMax.value) * 100)}%`
function conditionText(x: Threshold): string {
  const what = t(`fin.metric_${x.metric}`)
  const ccy = detail.value?.cost.currency ?? ''
  return x.mode === 'percent' ? t('fin.condPercent', { what, n: x.percent }) : t('fin.condAbsolute', { what, amount: m(x.amount_minor, ccy) })
}
function statusText(x: Threshold): string {
  if (x.alert_state === 'open') return t('fin.firedOpen', { at: fmtDate.value.format(new Date(x.alert_fired_at!)) })
  if (x.alert_state === 'acknowledged') return t('fin.firedAcked', { at: fmtDate.value.format(new Date(x.alert_fired_at!)) })
  const lim = limitOf(x)
  if (lim === null) return t('fin.inactive')
  const obs = x.metric === 'forecast' ? detail.value!.cost.forecast_minor : detail.value!.cost.spent_minor
  return t('fin.notFired', { gap: m(Math.max(0, lim - obs), detail.value!.cost.currency) })
}
function recipients(x: Threshold): string {
  return [x.notify_finance && t('org.finance'), x.notify_pm && t('work.owner'), x.notify_admins && t('org.admin')].filter(Boolean).join('、')
}

const th = reactive({ metric: 'labour_cost', mode: 'percent', percent: 80, amount: '', level: 'warn', notify_pm: true, notify_finance: true, notify_admins: false })
const addThreshold = () => run(() => api.post(`/finance/projects/${projectId.value}/thresholds`, {
  metric: th.metric, mode: th.mode, level: th.level, notify_pm: th.notify_pm, notify_finance: th.notify_finance, notify_admins: th.notify_admins,
  percent: th.mode === 'percent' ? Number(th.percent) : null,
  amount_minor: th.mode === 'absolute' ? toMinor(th.amount, detail.value!.cost.currency) : null,
}), t('work.saved'))
const delThreshold = (x: Threshold) => run(() => api.del(`/finance/thresholds/${x.id}`))
const ackNote = reactive<Record<string, string>>({})
const ack = (a: Alert) => run(() => api.post(`/alerts/${a.id}/ack`, { note: ackNote[a.id] ?? '' }), t('fin.acked'))
const budget = ref('')
const saveBudget = () => run(() => api.put(`/finance/projects/${projectId.value}/budget`, { labour_budget_minor: budget.value === '' ? null : toMinor(budget.value, detail.value!.cost.currency) }), t('work.saved'))

const rf = reactive({ user_id: '', amount: '', currency: 'CNY', effective_from: new Date().toISOString().slice(0, 10), note: '' })
const lastAffected = ref<{ affected: number; from: string } | null>(null)
const addRate = () => run(async () => {
  const r = await api.post<{ filled: number; affected: number }>('/finance/rates', { user_id: rf.user_id, rate_minor: toMinor(rf.amount, rf.currency), currency: rf.currency, effective_from: rf.effective_from, note: rf.note })
  lastAffected.value = r.affected ? { affected: r.affected, from: rf.effective_from } : null
  return t('fin.rateSaved', { filled: r.filled })
}, t('work.saved'))
const recalc = (from: string) => run(async () => {
  const r = await api.post<{ entries: number }>('/finance/recalculate', { from })
  lastAffected.value = null
  return t('fin.recalculated', { n: r.entries })
}, t('work.saved'))
const currentRates = computed(() => {
  const seen = new Set<string>()
  return rates.value.filter((r) => (seen.has(r.user_id) ? false : (seen.add(r.user_id), true)))
})

const ff = reactive({ from_ccy: 'CNY', to_ccy: 'USD', rate: '', effective_date: new Date().toISOString().slice(0, 10) })
const addFx = () => run(async () => {
  const r = await api.post<{ filled: number }>('/finance/fx', { ...ff })
  return t('fin.fxSaved', { filled: r.filled })
}, t('work.saved'))
const pf = reactive({ starts_on: '', ends_on: '' })
const closePeriod = () => run(async () => {
  const r = await api.post<{ locked: number }>('/finance/periods', { ...pf })
  return t('fin.periodClosed', { n: r.locked })
}, t('work.saved'))
const ex = reactive({ from: new Date(Date.now() - 30 * 864e5).toISOString().slice(0, 10), to: new Date().toISOString().slice(0, 10) })
const exportUrl = computed(() => `/api/v1/export/timesheet.csv?from=${ex.from}&to=${ex.to}${projectId.value ? `&project=${projectId.value}` : ''}`)
</script>

<template>
  <Shell>
    <div class="stack">
      <h1>{{ t('nav.finance') }}</h1>
      <div role="tablist" class="tabs" :aria-label="t('nav.finance')">
        <button v-for="k in TABS" :key="k" role="tab" type="button" :aria-selected="tab === k" :class="{ on: tab === k }" @click="tab = k">{{ t(`fin.tab_${k}`) }}</button>
      </div>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>

      <template v-if="tab === 'costs' && overview">
        <div v-for="a in overview.alerts.filter((x) => x.state === 'open')" :key="a.id" role="alert" class="notice row" :class="a.level === 'critical' ? 'error' : 'warn'">
          <span style="flex: 1 1 300px"><strong>{{ t(`fin.level_${a.level}`) }}</strong> · {{ a.project_key }} {{ a.project_name }} · {{ t(`fin.metric_${a.metric}`) }} {{ m(a.observed_minor, a.currency) }} ≥ {{ m(a.limit_minor, a.currency) }} · {{ fmtDate.format(new Date(a.fired_at)) }}</span>
          <input v-model="ackNote[a.id]" :placeholder="t('fin.ackNote')" :aria-label="t('fin.ackNote')" />
          <button type="button" @click="ack(a)">{{ t('fin.ack') }}</button>
        </div>
        <p v-for="p in overview.pending_fx" :key="p.from + p.to" class="notice warn row">
          <span style="flex: 1 1 300px">{{ t('fin.pendingFx', { n: p.entries, from: p.from, to: p.to, date: p.earliest }) }}</span>
          <button type="button" @click="tab = 'fx'; ff.from_ccy = p.from; ff.to_ccy = p.to; ff.effective_date = p.earliest">{{ t('fin.enterRate') }}</button>
        </p>
        <p v-for="r in overview.missing_rates" :key="r.user_name" class="notice warn row">
          <span style="flex: 1 1 300px">{{ t('fin.missingRate', { n: r.entries, name: r.user_name, date: r.earliest }) }}</span>
          <button type="button" @click="tab = 'rates'">{{ t('fin.setRate') }}</button>
        </p>

        <div class="row">
          <label class="field" style="flex: 1 1 260px">{{ t('work.projects') }}
            <select v-model="projectId"><option v-for="p in overview.projects" :key="p.project.id" :value="p.project.id">{{ p.project.key }} {{ p.project.name }} · {{ p.cost.used_pct ?? '—' }}%</option></select>
          </label>
          <label class="field">{{ t('fin.exportFrom') }}<input v-model="ex.from" type="date" /></label>
          <label class="field">{{ t('fin.exportTo') }}<input v-model="ex.to" type="date" /></label>
          <a class="button-link" :href="exportUrl" download>{{ t('fin.export') }}</a>
        </div>
        <p v-if="!overview.projects.length" class="empty">{{ t('work.noProjects') }}</p>

        <template v-if="detail">
          <div class="row" style="align-items: stretch">
            <section class="card stack" style="flex: 2 1 420px">
              <div class="row" style="justify-content: space-between">
                <div><div class="muted small">{{ t('fin.spent') }}</div><div class="big">{{ m(detail.cost.spent_minor, detail.cost.currency) }}</div></div>
                <div style="text-align: right"><div class="muted small">{{ t('work.budget') }}</div><div class="mid">{{ m(detail.cost.budget_minor, detail.cost.currency) }}</div></div>
              </div>
              <div class="bar" role="img" :aria-label="t('fin.barLabel', { pct: detail.cost.used_pct ?? '—' })">
                <div class="fill" :style="{ width: pct(detail.cost.spent_minor) }" />
                <div class="forecast" :style="{ left: pct(detail.cost.forecast_minor) }" :title="t('fin.forecast')" />
                <div v-for="x in detail.thresholds" :key="x.id" class="mark" :class="`m-${x.level}`" :style="{ left: pct(limitOf(x) ?? 0) }" :title="conditionText(x)" />
              </div>
              <div class="row small muted">
                <span>{{ t('fin.used', { pct: detail.cost.used_pct ?? '—' }) }}</span>
                <span>{{ t('fin.forecastText', { amount: m(detail.cost.forecast_minor, detail.cost.currency) }) }}</span>
                <span>{{ t('fin.costedHours', { h: hours(detail.cost.costed_minutes), r: hours(detail.cost.remaining_minutes) }) }}</span>
                <span v-if="detail.cost.uncosted_entries" class="warn-text">{{ t('fin.uncosted', { n: detail.cost.uncosted_entries }) }}</span>
              </div>
              <form class="row" @submit.prevent="saveBudget">
                <label class="field">{{ t('work.budget') }} ({{ detail.cost.currency }})<input v-model="budget" type="number" min="0" step="any" /></label>
                <button type="submit">{{ t('common.save') }}</button>
              </form>
            </section>
            <section class="card stack" style="flex: 1 1 280px">
              <div class="muted small">{{ t('fin.byPerson') }}</div>
              <ul class="people">
                <li v-for="p in detail.people" :key="p.user_id"><span>{{ p.user_name }}<span v-if="p.rate_minor !== null" class="muted small"> · {{ m(p.rate_minor, p.rate_currency ?? '') }}/{{ t('fin.hour') }}</span></span><span>{{ m(p.cost_minor, detail.cost.currency) }} <span class="muted small">{{ hours(p.minutes) }}{{ t('work.hours') }}</span></span></li>
              </ul>
              <p v-if="!detail.people.length" class="empty">{{ t('work.noLogs') }}</p>
            </section>
          </div>

          <section class="card stack">
            <h2>{{ t('fin.thresholds') }}</h2>
            <div class="table-wrap">
              <table>
                <thead><tr><th>{{ t('fin.level') }}</th><th>{{ t('fin.condition') }}</th><th>{{ t('fin.notify') }}</th><th>{{ t('work.status') }}</th><th><span class="sr-only">{{ t('work.delete') }}</span></th></tr></thead>
                <tbody>
                  <tr v-for="x in detail.thresholds" :key="x.id">
                    <td><span class="pill" :class="`level-${x.level}`">{{ t(`fin.level_${x.level}`) }}</span></td>
                    <td>{{ conditionText(x) }}</td>
                    <td>{{ recipients(x) }}</td>
                    <td>{{ statusText(x) }}</td>
                    <td><button type="button" @click="delThreshold(x)">{{ t('work.delete') }}</button></td>
                  </tr>
                </tbody>
              </table>
            </div>
            <form class="row" @submit.prevent="addThreshold">
              <select v-model="th.level" :aria-label="t('fin.level')"><option v-for="l in ['info', 'warn', 'critical']" :key="l" :value="l">{{ t(`fin.level_${l}`) }}</option></select>
              <select v-model="th.metric" :aria-label="t('fin.metric')"><option value="labour_cost">{{ t('fin.metric_labour_cost') }}</option><option value="forecast">{{ t('fin.metric_forecast') }}</option></select>
              <select v-model="th.mode" :aria-label="t('fin.mode')"><option value="percent">{{ t('fin.mode_percent') }}</option><option value="absolute">{{ t('fin.mode_absolute') }}</option></select>
              <input v-if="th.mode === 'percent'" v-model.number="th.percent" type="number" min="1" max="1000" :aria-label="t('fin.mode_percent')" style="width: 90px" />
              <input v-else v-model="th.amount" type="number" min="0" step="any" :aria-label="t('fin.mode_absolute')" :placeholder="detail.cost.currency" style="width: 140px" />
              <label class="chk"><input v-model="th.notify_finance" type="checkbox" />{{ t('org.finance') }}</label>
              <label class="chk"><input v-model="th.notify_pm" type="checkbox" />{{ t('work.owner') }}</label>
              <label class="chk"><input v-model="th.notify_admins" type="checkbox" />{{ t('org.admin') }}</label>
              <button class="primary" type="submit">+ {{ t('fin.addThreshold') }}</button>
            </form>
            <p class="muted small">{{ t('fin.onceRule') }}</p>
          </section>

          <section v-if="detail.alerts.length" class="card stack">
            <h2>{{ t('fin.alertHistory') }}</h2>
            <ul class="people">
              <li v-for="a in detail.alerts" :key="a.id"><span><span class="pill" :class="`level-${a.level}`">{{ t(`fin.level_${a.level}`) }}</span> {{ m(a.observed_minor, a.currency) }} ≥ {{ m(a.limit_minor, a.currency) }}</span><span class="muted small">{{ t(`fin.state_${a.state}`) }} · {{ fmtDate.format(new Date(a.fired_at)) }}<template v-if="a.ack_by_name"> · {{ a.ack_by_name }} {{ a.ack_note }}</template></span></li>
            </ul>
          </section>
        </template>
      </template>

      <template v-if="tab === 'rates'">
        <form class="card row" @submit.prevent="addRate">
          <label class="field">{{ t('org.person') }}<select v-model="rf.user_id" required><option value="">{{ t('org.choose') }}</option><option v-for="x in members.filter((y) => y.status === 'active')" :key="x.user_id" :value="x.user_id">{{ x.display_name }}</option></select></label>
          <label class="field" style="flex: 0 1 140px">{{ t('fin.ratePerHour') }}<input v-model="rf.amount" type="number" min="0" step="any" required /></label>
          <label class="field" style="flex: 0 1 100px">{{ t('auth.currency') }}<input v-model="rf.currency" required pattern="[A-Z]{3}" maxlength="3" /></label>
          <label class="field">{{ t('fin.effectiveFrom') }}<input v-model="rf.effective_from" type="date" required /></label>
          <label class="field" style="flex: 1 1 160px">{{ t('fin.note') }}<input v-model="rf.note" maxlength="200" /></label>
          <button class="primary" type="submit">{{ t('common.save') }}</button>
        </form>
        <p v-if="lastAffected" class="notice warn row">
          <span style="flex: 1 1 300px">{{ t('fin.backdated', { n: lastAffected.affected, from: lastAffected.from }) }}</span>
          <button type="button" @click="recalc(lastAffected.from)">{{ t('fin.recalc') }}</button>
        </p>
        <section class="card stack">
          <h2>{{ t('fin.currentRates') }}</h2>
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('org.person') }}</th><th>{{ t('fin.ratePerHour') }}</th><th>{{ t('fin.effectiveFrom') }}</th><th>{{ t('fin.note') }}</th><th>{{ t('fin.setBy') }}</th></tr></thead>
              <tbody>
                <tr v-for="r in rates" :key="r.id" :class="{ old: !currentRates.includes(r) }">
                  <td>{{ r.user_name }}</td><td>{{ m(r.rate_minor, r.currency) }}</td><td>{{ r.effective_from }}</td><td>{{ r.note }}</td><td>{{ r.set_by_name }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <p class="muted small">{{ t('fin.ratesNote') }}</p>
        </section>
      </template>

      <template v-if="tab === 'fx'">
        <form class="card row" @submit.prevent="addFx">
          <label class="field" style="flex: 0 1 100px">{{ t('fin.fromCcy') }}<input v-model="ff.from_ccy" required pattern="[A-Z]{3}" maxlength="3" /></label>
          <label class="field" style="flex: 0 1 100px">{{ t('fin.toCcy') }}<input v-model="ff.to_ccy" required pattern="[A-Z]{3}" maxlength="3" /></label>
          <label class="field" style="flex: 0 1 160px">{{ t('fin.rate') }}<input v-model="ff.rate" required inputmode="decimal" pattern="[0-9]+(\.[0-9]+)?" /></label>
          <label class="field">{{ t('fin.effectiveFrom') }}<input v-model="ff.effective_date" type="date" required /></label>
          <button class="primary" type="submit">{{ t('common.save') }}</button>
        </form>
        <p class="muted small">{{ t('fin.fxHelp', { from: ff.from_ccy, to: ff.to_ccy }) }}</p>
        <p v-for="p in fx.pending" :key="p.from + p.to" class="notice warn">{{ t('fin.pendingFx', { n: p.entries, from: p.from, to: p.to, date: p.earliest }) }}</p>
        <section class="card">
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('fin.pair') }}</th><th>{{ t('fin.rate') }}</th><th>{{ t('fin.effectiveFrom') }}</th><th>{{ t('fin.setBy') }}</th></tr></thead>
              <tbody><tr v-for="r in fx.rates" :key="r.id"><td>{{ r.from_ccy }} → {{ r.to_ccy }}</td><td>{{ r.rate }}</td><td>{{ r.effective_date }}</td><td>{{ r.set_by_name }}</td></tr></tbody>
            </table>
          </div>
        </section>
      </template>

      <template v-if="tab === 'periods'">
        <form class="card row" @submit.prevent="closePeriod">
          <label class="field">{{ t('work.start') }}<input v-model="pf.starts_on" type="date" required /></label>
          <label class="field">{{ t('work.end') }}<input v-model="pf.ends_on" type="date" required /></label>
          <button class="primary" type="submit">{{ t('fin.closePeriod') }}</button>
        </form>
        <p class="muted small">{{ t('fin.periodHelp') }}</p>
        <section class="card">
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('fin.period') }}</th><th>{{ t('work.status') }}</th><th>{{ t('fin.entries') }}</th><th>{{ t('fin.closedBy') }}</th></tr></thead>
              <tbody><tr v-for="p in periods" :key="p.id"><td>{{ p.starts_on }} – {{ p.ends_on }}</td><td>{{ t(`fin.p_${p.status}`) }}</td><td>{{ p.entries }}</td><td>{{ p.closed_by_name }} <span class="muted small">{{ p.closed_at ? fmtDate.format(new Date(p.closed_at)) : '' }}</span></td></tr></tbody>
            </table>
          </div>
        </section>
      </template>

      <template v-if="tab === 'audit'">
        <section class="card">
          <div class="table-wrap">
            <table>
              <thead><tr><th>{{ t('fin.when') }}</th><th>{{ t('org.person') }}</th><th>{{ t('fin.action') }}</th><th>{{ t('fin.details') }}</th></tr></thead>
              <tbody><tr v-for="a in audit" :key="a.id"><td>{{ fmtDate.format(new Date(a.created_at)) }}</td><td>{{ a.actor_name }}</td><td>{{ a.action }}</td><td class="small"><code>{{ JSON.stringify(a.payload) }}</code></td></tr></tbody>
            </table>
          </div>
        </section>
      </template>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
.big { font-size: 26px; font-weight: 700; }
.mid { font-size: 18px; }
.bar { position: relative; height: 14px; border-radius: 7px; background: var(--inset); margin: 8px 0 4px; }
.fill { height: 14px; border-radius: 7px; background: var(--accent); }
.forecast { position: absolute; top: 2px; width: 10px; height: 10px; margin-left: -5px; border-radius: 50%; border: 2px solid var(--accent); background: var(--surface); }
.mark { position: absolute; top: -4px; width: 2px; height: 22px; margin-left: -1px; }
.m-info { background: var(--info); }
.m-warn { background: var(--warn); }
.m-critical { background: var(--danger); }
.people { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.people li { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 8px; }
.chk { display: inline-flex; align-items: center; gap: 6px; }
.chk input { height: auto; }
.warn-text { color: var(--warn); }
.old td { color: var(--text-3); }
.button-link { display: inline-flex; align-items: center; height: 40px; padding: 0 14px; border: 1px solid var(--line-strong); border-radius: 8px; color: var(--text); text-decoration: none; align-self: flex-end; }
.sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
code { font-size: 12px; overflow-wrap: anywhere; white-space: normal; }
</style>
