<script setup lang="ts">
// "Who is working on what": people × weeks, stacked by project, plus one
// person's breakdown across projects, routine work and requests.
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import { api } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'
import { money } from '../lib/money'
import { currentMembership } from '../lib/session'

interface Person { user_id: string; name: string; departments: string | null; open_items: number; open_estimate_minutes: number }
interface Cell { user_id: string; week: string; bucket: string; minutes: number }
interface Now { user_id: string; key: string | null; title: string; project_key: string | null }
interface Breakdown {
  by_project: { project_id: string | null; key: string | null; name: string | null; minutes: number; cost_minor?: number; currency?: string | null }[]
  by_kind: { kind: string; minutes: number }[]; by_day: { day: string; minutes: number }[]; items: { key: string | null; title: string; minutes: number }[]
}

const { t, locale } = useI18n()
const iso = (d: Date) => d.toISOString().slice(0, 10)
function monday(d: Date) {
  const x = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()))
  x.setUTCDate(x.getUTCDate() - ((x.getUTCDay() + 6) % 7))
  return x
}
const range = reactive({ from: iso(new Date(monday(new Date()).getTime() - 21 * 864e5)), to: iso(new Date()) })
const people = ref<Person[]>([])
const cells = ref<Cell[]>([])
const now = ref<Now[]>([])
const error = ref('')
const picked = ref<Person | null>(null)
const breakdown = ref<Breakdown | null>(null)
const sees = computed(() => !!(currentMembership()?.is_admin || currentMembership()?.is_finance))

async function load() {
  error.value = ''
  try {
    const d = await api.get<{ data: { people: Person[]; cells: Cell[]; in_progress: Now[] } }>(`/reports/workload?from=${range.from}&to=${range.to}`)
    people.value = d.data.people
    cells.value = d.data.cells
    now.value = d.data.in_progress
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(load)

const weeks = computed(() => {
  const out: string[] = []
  let d = monday(new Date(range.from))
  const end = new Date(range.to).getTime()
  while (d.getTime() <= end) {
    out.push(iso(d))
    d = new Date(d.getTime() + 7 * 864e5)
  }
  return out
})
const buckets = computed(() => [...new Set(cells.value.map((c) => c.bucket))].sort())
const PALETTE = ['#2E5D4A', '#2A3C5C', '#806332', '#9E2F2F', '#5B6B3A', '#6A4C7D', '#3E7C8C', '#8C5A3E']
const color = (b: string) => (b === 'routine' ? '#867E76' : b === 'request' ? '#BBB5AC' : PALETTE[buckets.value.filter((x) => x !== 'routine' && x !== 'request').indexOf(b) % PALETTE.length])
const label = (b: string) => (b === 'routine' || b === 'request' ? t(`flow.group_${b}`) : b)
const WEEK_CAP = 40 * 60
function cell(u: string, w: string) {
  const list = cells.value.filter((c) => c.user_id === u && c.week === w)
  const total = list.reduce((a, c) => a + c.minutes, 0)
  return { list, total }
}
const nowFor = (u: string) => now.value.filter((n) => n.user_id === u)
const fmtWeek = (w: string) => new Intl.DateTimeFormat(locale.value, { month: 'numeric', day: 'numeric', timeZone: 'UTC' }).format(new Date(w))

async function pick(p: Person) {
  picked.value = p
  breakdown.value = null
  try {
    breakdown.value = (await api.get<{ data: Breakdown }>(`/reports/people/${p.user_id}?from=${range.from}&to=${range.to}`)).data
  } catch (e) {
    error.value = errorText(e, t)
  }
}
// Every day of the range, so a single busy day does not fill the whole chart.
const days = computed(() => {
  const have = new Map((breakdown.value?.by_day ?? []).map((d) => [d.day.slice(0, 10), d.minutes]))
  const out: { day: string; minutes: number }[] = []
  for (let d = new Date(range.from).getTime(); d <= new Date(range.to).getTime(); d += 864e5) {
    const k = iso(new Date(d))
    out.push({ day: k, minutes: have.get(k) ?? 0 })
  }
  return out
})
const maxDay = computed(() => Math.max(60, ...(breakdown.value?.by_day.map((d) => d.minutes) ?? [])))
const totalOf = (b: Breakdown) => b.by_project.reduce((a, x) => a + x.minutes, 0)
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('nav.team') }}</h1>
        <form class="row" @submit.prevent="load">
          <label class="field">{{ t('fin.exportFrom') }}<input v-model="range.from" type="date" /></label>
          <label class="field">{{ t('fin.exportTo') }}<input v-model="range.to" type="date" /></label>
          <button type="submit" style="align-self: flex-end">{{ t('rep.refresh') }}</button>
        </form>
      </div>
      <p class="muted">{{ t('rep.teamIntro') }}</p>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <div class="legend">
        <span v-for="b in buckets" :key="b"><i :style="{ background: color(b) }" />{{ label(b) }}</span>
      </div>
      <div class="layout" :class="{ withPanel: !!picked }">
        <div class="card table-wrap">
          <table class="grid">
            <thead>
              <tr><th>{{ t('org.person') }}</th><th v-for="w in weeks" :key="w">{{ t('rep.weekOf', { d: fmtWeek(w) }) }}</th><th>{{ t('rep.openLoad') }}</th></tr>
            </thead>
            <tbody>
              <tr v-for="p in people" :key="p.user_id" :class="{ on: picked?.user_id === p.user_id }">
                <th scope="row">
                  <button type="button" class="who" @click="pick(p)">{{ p.name }}</button>
                  <div class="muted small">{{ p.departments ?? '' }}</div>
                  <div v-for="n in nowFor(p.user_id).slice(0, 2)" :key="n.title" class="small now">▶ {{ n.key }} {{ n.title }}</div>
                </th>
                <td v-for="w in weeks" :key="w">
                  <div class="stackbar" :title="cell(p.user_id, w).list.map((c) => `${label(c.bucket)} ${hours(c.minutes)}h`).join(' · ')">
                    <span v-for="c in cell(p.user_id, w).list" :key="c.bucket" :style="{ width: `${Math.min(100, (c.minutes / WEEK_CAP) * 100)}%`, background: color(c.bucket) }" />
                  </div>
                  <div class="small" :class="{ over: cell(p.user_id, w).total > WEEK_CAP }">{{ cell(p.user_id, w).total ? `${hours(cell(p.user_id, w).total)}h` : '' }}</div>
                </td>
                <td class="small">{{ p.open_items }} · {{ hours(p.open_estimate_minutes) }}h</td>
              </tr>
            </tbody>
          </table>
        </div>
        <aside v-if="picked" class="card stack panel">
          <div class="row" style="justify-content: space-between">
            <h2>{{ picked.name }}</h2>
            <button type="button" :aria-label="t('work.close')" @click="picked = null">✕</button>
          </div>
          <template v-if="breakdown">
            <div class="muted small">{{ range.from }} – {{ range.to }} · {{ hours(totalOf(breakdown)) }}{{ t('work.hours') }}</div>
            <h3>{{ t('rep.byProject') }}</h3>
            <ul class="rows">
              <li v-for="x in breakdown.by_project" :key="x.key ?? 'none'"><span>{{ x.key ?? t('rep.noProject') }} {{ x.name ?? '' }}</span><span>{{ hours(x.minutes) }}h<template v-if="sees && x.cost_minor !== undefined && x.currency"> · {{ money(x.cost_minor, x.currency, locale) }}</template></span></li>
            </ul>
            <h3>{{ t('rep.byKind') }}</h3>
            <ul class="rows">
              <li v-for="x in breakdown.by_kind" :key="x.kind"><span>{{ t(`flow.group_${x.kind}`) }}</span><span>{{ hours(x.minutes) }}h · {{ Math.round((x.minutes / Math.max(1, totalOf(breakdown))) * 100) }}%</span></li>
            </ul>
            <h3>{{ t('rep.byDay') }}</h3>
            <div class="days" role="img" :aria-label="t('rep.byDay')">
              <span v-for="d in days" :key="d.day" class="day" :title="`${d.day} ${hours(d.minutes)}h`"><i :style="{ height: `${(d.minutes / maxDay) * 100}%` }" /></span>
            </div>
            <h3>{{ t('rep.topItems') }}</h3>
            <ul class="rows">
              <li v-for="x in breakdown.items" :key="(x.key ?? '') + x.title"><span>{{ x.key }} {{ x.title }}</span><span>{{ hours(x.minutes) }}h</span></li>
            </ul>
          </template>
          <p v-else class="muted">{{ t('common.loading') }}</p>
        </aside>
      </div>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
h3 { font-size: 14px; margin: 4px 0 0; }
.legend { display: flex; flex-wrap: wrap; gap: 12px; font-size: 13px; }
.legend i { display: inline-block; width: 12px; height: 12px; border-radius: 3px; margin-right: 6px; vertical-align: -1px; }
.grid th, .grid td { vertical-align: top; }
.grid td { min-width: 96px; }
.grid tr.on th { color: var(--accent); }
.who { height: auto; padding: 0; border: 0; background: none; color: inherit; font: inherit; font-weight: 600; text-align: left; cursor: pointer; text-decoration: underline; text-decoration-color: var(--line); }
.now { color: var(--accent); max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.stackbar { display: flex; height: 12px; border-radius: 4px; background: var(--inset); overflow: hidden; }
.over { color: var(--danger); font-weight: 600; }
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(300px, 380px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
.rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; font-size: 13px; }
.rows li { display: flex; justify-content: space-between; gap: 8px; }
.rows li span:first-child { min-width: 0; overflow-wrap: anywhere; }
.days { display: flex; align-items: flex-end; gap: 2px; height: 60px; border-bottom: 1px solid var(--line); }
.day { flex: 1 1 0; height: 100%; display: flex; align-items: flex-end; }
.day i { display: block; width: 100%; background: var(--accent); border-radius: 2px 2px 0 0; }
</style>
