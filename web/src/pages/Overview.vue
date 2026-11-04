<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import { api, type Item, type Project } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours, toDay, todayDay } from '../lib/dates'
import { money } from '../lib/money'

const { t, locale } = useI18n()
const projects = ref<Project[]>([])
const handoffs = ref<{ passed_more_than_3: Item[]; avg_hours_to_accept: number | null }>({ passed_more_than_3: [], avg_hours_to_accept: null })
const error = ref('')

onMounted(async () => {
  try {
    const d = await api.get<{ projects: Project[]; handoffs: typeof handoffs.value }>('/reports/portfolio')
    projects.value = d.projects
    handoffs.value = d.handoffs
  } catch (e) {
    error.value = errorText(e, t)
  }
})

const dated = computed(() => projects.value.filter((p) => p.planned_start && p.planned_end))
const span = computed(() => {
  const days = dated.value.flatMap((p) => [toDay(p.planned_start!), toDay(p.planned_end!)])
  const today = todayDay()
  const lo = Math.min(today - 7, ...days)
  const hi = Math.max(today + 14, ...days)
  return { lo, hi, len: Math.max(1, hi - lo + 1) }
})
const left = (d: string) => `${((toDay(d) - span.value.lo) / span.value.len) * 100}%`
const width = (a: string, b: string) => `${((toDay(b) - toDay(a) + 1) / span.value.len) * 100}%`
const todayLeft = computed(() => `${((todayDay() - span.value.lo) / span.value.len) * 100}%`)
const months = computed(() => {
  const out: { label: string; left: string }[] = []
  const d = new Date((span.value.lo) * 864e5)
  d.setUTCDate(1)
  const f = new Intl.DateTimeFormat(locale.value, { month: 'short', year: '2-digit', timeZone: 'UTC' })
  while (d.getTime() / 864e5 <= span.value.hi) {
    const day = Math.floor(d.getTime() / 864e5)
    if (day >= span.value.lo) out.push({ label: f.format(d), left: `${((day - span.value.lo) / span.value.len) * 100}%` })
    d.setUTCMonth(d.getUTCMonth() + 1)
  }
  return out
})
const progress = (p: Project) => (p.estimate_minutes ? Math.min(100, Math.round((p.logged_minutes / p.estimate_minutes) * 100)) : 0)
function health(p: Project): 'ok' | 'warn' | 'bad' {
  if (p.open_alerts > 0) return 'bad'
  const used = p.budget_used_pct ?? 0
  if (used >= 80 || (p.planned_end && toDay(p.planned_end) < todayDay() && p.open_items > 0)) return 'warn'
  return 'ok'
}
</script>

<template>
  <Shell>
    <div class="stack">
      <h1>{{ t('nav.overview') }}</h1>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <div class="grid-auto">
        <div class="card"><div class="muted small">{{ t('rep.activeProjects') }}</div><div class="big">{{ projects.filter((p) => p.status === 'active').length }}</div></div>
        <div class="card"><div class="muted small">{{ t('rep.openItems') }}</div><div class="big">{{ projects.reduce((a, p) => a + p.open_items, 0) }}</div></div>
        <div class="card"><div class="muted small">{{ t('rep.loggedHours') }}</div><div class="big">{{ hours(projects.reduce((a, p) => a + p.logged_minutes, 0)) }}</div></div>
        <div class="card"><div class="muted small">{{ t('rep.openAlerts') }}</div><div class="big" :class="{ bad: projects.some((p) => p.open_alerts) }">{{ projects.reduce((a, p) => a + p.open_alerts, 0) }}</div></div>
        <div class="card"><div class="muted small">{{ t('rep.avgAccept') }}</div><div class="big">{{ handoffs.avg_hours_to_accept === null ? '—' : handoffs.avg_hours_to_accept.toFixed(1) }}<span class="muted small"> {{ t('work.hours') }}</span></div></div>
      </div>

      <section v-if="dated.length" class="card stack">
        <h2>{{ t('rep.timeline') }}</h2>
        <div class="tl" role="img" :aria-label="t('rep.timeline')">
          <div class="tl-head"><span v-for="m in months" :key="m.left" class="month" :style="{ left: m.left }">{{ m.label }}</span></div>
          <div class="tl-body">
            <div class="today" :style="{ left: todayLeft }" />
            <RouterLink v-for="p in dated" :key="p.id" :to="`/projects/${p.id}`" class="tl-row">
              <span class="tl-name">{{ p.key }} {{ p.name }}</span>
              <span class="tl-track"><span class="tl-bar" :class="`h-${health(p)}`" :style="{ left: left(p.planned_start!), width: width(p.planned_start!, p.planned_end!) }"><span class="tl-fill" :style="{ width: `${progress(p)}%` }" /></span></span>
            </RouterLink>
          </div>
        </div>
      </section>

      <section class="card stack">
        <h2>{{ t('rep.portfolio') }}</h2>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('work.projects') }}</th><th>{{ t('work.owner') }}</th><th>{{ t('work.start') }} – {{ t('work.end') }}</th><th>{{ t('rep.hours') }}</th><th>{{ t('rep.items') }}</th><th>{{ t('rep.budgetUsed') }}</th><th>{{ t('rep.health') }}</th></tr></thead>
            <tbody>
              <tr v-for="p in projects" :key="p.id">
                <td><RouterLink :to="`/projects/${p.id}`">{{ p.key }} {{ p.name }}</RouterLink></td>
                <td>{{ p.owner_name }}</td>
                <td>{{ p.planned_start ?? '—' }} – {{ p.planned_end ?? '—' }}</td>
                <td>{{ hours(p.logged_minutes) }} / {{ hours(p.estimate_minutes) }}</td>
                <td>{{ p.open_items }} / {{ p.total_items }}</td>
                <td>{{ p.budget_used_pct === null ? '—' : `${p.budget_used_pct}%` }}<span v-if="p.cost" class="muted small"> · {{ money(p.cost.spent_minor, p.currency, locale) }}</span></td>
                <td><span class="pill" :class="health(p) === 'bad' ? 'level-critical' : health(p) === 'warn' ? 'level-warn' : 'level-info'">{{ t(`rep.h_${health(p)}`) }}</span></td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section v-if="handoffs.passed_more_than_3.length" class="card stack">
        <h2>{{ t('rep.pingPong') }}</h2>
        <ul class="plain">
          <li v-for="i in handoffs.passed_more_than_3" :key="i.id">{{ i.key }} {{ i.title }} · {{ t('flow.handoffCount', { n: i.handoff_count }) }} · {{ i.holder_name }}</li>
        </ul>
      </section>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
.big { font-size: 26px; font-weight: 700; margin-top: 4px; }
.big.bad { color: var(--danger); }
.tl { display: flex; flex-direction: column; gap: 6px; overflow: hidden; }
.tl-head { position: relative; height: 20px; margin-left: 200px; border-bottom: 1px solid var(--line); }
.month { position: absolute; font-size: 12px; color: var(--text-3); white-space: nowrap; }
.tl-body { position: relative; display: flex; flex-direction: column; gap: 6px; }
.today { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--danger); margin-left: 200px; transform: translateX(-1px); z-index: 1; pointer-events: none; }
.tl-row { display: flex; align-items: center; gap: 0; color: var(--text); text-decoration: none; min-height: 30px; }
.tl-name { flex: 0 0 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-right: 8px; font-size: 13px; }
.tl-track { position: relative; flex: 1 1 auto; height: 18px; }
.tl-bar { position: absolute; top: 0; height: 18px; border-radius: 5px; background: var(--accent-soft); border: 1px solid var(--accent); overflow: hidden; }
.tl-bar.h-warn { border-color: var(--warn); background: var(--warn-bg); }
.tl-bar.h-bad { border-color: var(--danger); background: var(--danger-bg); }
.tl-fill { display: block; height: 100%; background: var(--accent); opacity: 0.55; }
.plain { margin: 0; padding-left: 18px; }
@media (max-width: 640px) { .tl-name { flex-basis: 110px; } .tl-head, .today { margin-left: 110px; } }
</style>
