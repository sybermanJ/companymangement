<script setup lang="ts">
// Approval routes: who approves each kind of request and contracts, with amount tiers.
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Member } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { money, toMajor, toMinor } from '../../lib/money'
import { ROUTE_SUBJECTS, STEP_KINDS, type Route } from '../../lib/lifecycle'

const props = defineProps<{ members: Member[]; isAdmin: boolean; currency: string }>()
const { t, locale } = useI18n()
const routes = ref<Route[]>([])
const error = ref('')
const notice = ref('')
const editing = ref<string | null>(null)
type Step = { kind: string; user_id: string; label: string }
const f = reactive({ subject: 'reimbursement', name: '', min: '', steps: [{ kind: 'phase_manager', user_id: '', label: '' }, { kind: 'finance', user_id: '', label: '' }] as Step[], active: true })

async function load() {
  try {
    routes.value = await api.get<Route[]>('/approval-routes')
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(load)
const nameOf = (id?: string | null) => props.members.find((m) => m.user_id === id)?.display_name ?? '?'
const stepText = (s: Route['steps'][number]) => (s.kind === 'user' ? nameOf(s.user_id) : s.label || t(`lc.step_${s.kind}`))
const grouped = computed(() => ROUTE_SUBJECTS.map((s) => ({ subject: s, routes: routes.value.filter((r) => r.subject === s) })))

function edit(r: Route | null, subject?: string) {
  editing.value = r?.id ?? 'new'
  Object.assign(f, {
    subject: r?.subject ?? subject ?? 'reimbursement', name: r?.name ?? '', min: r ? String(toMajor(r.min_amount_minor, props.currency)) : '',
    steps: (r?.steps ?? [{ kind: 'phase_manager' }, { kind: 'finance' }]).map((s) => ({ kind: s.kind, user_id: s.user_id ?? '', label: s.label ?? '' })),
    active: r?.active ?? true,
  })
}
async function save() {
  error.value = ''
  notice.value = ''
  const body = { subject: f.subject, name: f.name || t('lc.routeDefaultName'), min_amount_minor: f.min ? toMinor(f.min, props.currency) : 0, steps: f.steps.map((s) => ({ kind: s.kind, user_id: s.kind === 'user' ? s.user_id : null, label: s.label })), active: f.active }
  try {
    if (editing.value === 'new') await api.post('/approval-routes', body)
    else await api.put(`/approval-routes/${editing.value}`, body)
    editing.value = null
    notice.value = t('work.saved')
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
async function remove(r: Route) {
  try {
    await api.del(`/approval-routes/${r.id}`)
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
const move = (i: number, d: number) => { const s = f.steps.splice(i, 1)[0]; f.steps.splice(i + d, 0, s) }
</script>

<template>
  <section class="card stack" role="tabpanel">
    <p class="muted">{{ t('lc.routesIntro') }}</p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
    <form v-if="editing" class="card inset stack" @submit.prevent="save">
      <div class="row">
        <label class="field">{{ t('lc.routeFor') }}<select v-model="f.subject"><option v-for="s in ROUTE_SUBJECTS" :key="s" :value="s">{{ t(`lc.k_${s}`) }}</option></select></label>
        <label class="field grow">{{ t('lc.routeName') }}<input v-model="f.name" maxlength="100" :placeholder="t('lc.routeNamePh')" /></label>
        <label class="field">{{ t('lc.fromAmount', { ccy: currency }) }}<input v-model="f.min" type="number" min="0" step="0.01" placeholder="0" /></label>
        <label class="check" style="align-self: flex-end"><input v-model="f.active" type="checkbox" /> {{ t('flow.active') }}</label>
      </div>
      <ol class="steps-edit">
        <li v-for="(s, i) in f.steps" :key="i" class="row">
          <span class="muted">{{ i + 1 }}.</span>
          <select v-model="s.kind" :aria-label="t('lc.stepKind')"><option v-for="k in STEP_KINDS" :key="k" :value="k">{{ t(`lc.step_${k}`) }}</option></select>
          <select v-if="s.kind === 'user'" v-model="s.user_id" required :aria-label="t('org.person')"><option value="" disabled>{{ t('org.choose') }}</option><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
          <input v-model="s.label" maxlength="60" :placeholder="t('lc.stepLabel')" :aria-label="t('lc.stepLabel')" />
          <button type="button" :disabled="i === 0" :aria-label="t('lc.up')" @click="move(i, -1)">↑</button>
          <button type="button" :disabled="i === f.steps.length - 1" :aria-label="t('lc.down')" @click="move(i, 1)">↓</button>
          <button type="button" :disabled="f.steps.length === 1" :aria-label="t('work.delete')" @click="f.steps.splice(i, 1)">×</button>
        </li>
      </ol>
      <div class="row">
        <button type="button" @click="f.steps.push({ kind: 'finance', user_id: '', label: '' })">+ {{ t('lc.addStep') }}</button>
        <span class="spacer" />
        <button type="button" @click="editing = null">{{ t('common.cancel') }}</button>
        <button class="primary" type="submit">{{ t('common.save') }}</button>
      </div>
      <p class="muted small">{{ t('lc.stepsHint') }}</p>
    </form>
    <div class="table-wrap">
      <table>
        <thead><tr><th>{{ t('lc.routeFor') }}</th><th>{{ t('lc.fromAmountShort') }}</th><th>{{ t('lc.route') }}</th><th /></tr></thead>
        <tbody>
          <template v-for="g in grouped" :key="g.subject">
            <tr v-for="(r, i) in g.routes" :key="r.id" :class="{ off: !r.active }">
              <td><strong v-if="i === 0">{{ t(`lc.k_${g.subject}`) }}</strong></td>
              <td>{{ r.min_amount_minor ? `≥ ${money(r.min_amount_minor, currency, locale)}` : t('lc.anyAmount') }}<div class="muted small">{{ r.name === 'default' ? t('lc.routeDefaultName') : r.name }}<template v-if="!r.active"> · {{ t('flow.paused') }}</template></div></td>
              <td><span v-for="(s, j) in r.steps" :key="j"><span v-if="j" class="muted"> → </span><span class="badge">{{ stepText(s) }}</span></span></td>
              <td class="actions"><template v-if="isAdmin"><button type="button" @click="edit(r)">{{ t('lc.edit') }}</button> <button v-if="r.min_amount_minor" type="button" @click="remove(r)">{{ t('work.delete') }}</button></template></td>
            </tr>
            <tr v-if="isAdmin"><td /><td colspan="3"><button type="button" class="link" @click="edit(null, g.subject)">+ {{ t('lc.addTier') }}</button></td></tr>
          </template>
        </tbody>
      </table>
    </div>
  </section>
</template>

<style scoped>
.grow { flex: 1 1 200px; }
.inset { background: var(--inset); }
.steps-edit { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.spacer { flex: 1 1 auto; }
.check { display: inline-flex; align-items: center; gap: 6px; }
.check input { height: auto; }
tr.off { opacity: .6; }
.actions { white-space: nowrap; text-align: right; }
button.link { height: auto; padding: 0; border: 0; background: none; color: var(--accent); }
</style>
