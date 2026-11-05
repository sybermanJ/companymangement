<script setup lang="ts">
// New or returned request: kind, project and phase, reason, line items, trip details, receipts.
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Item, type Member, type Project } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { currentMembership } from '../../lib/session'
import { money, toMajor, toMinor } from '../../lib/money'
import { CLAIM_KINDS, LINE_CATEGORIES, uploadDocs, type Claim, type Line } from '../../lib/lifecycle'

const props = defineProps<{ members: Member[]; claim?: Claim | null; lines?: Line[]; initial?: { kind?: string; parent?: string; project?: string; phase?: string } }>()
const emit = defineEmits<{ saved: [id: string]; cancel: [] }>()
const { t, locale } = useI18n()
const projects = ref<Project[]>([])
const phases = ref<Item[]>([])
const error = ref('')
const busy = ref(false)
const files = ref<File[]>([])

const c = props.claim
const f = reactive({
  kind: c?.kind ?? props.initial?.kind ?? 'reimbursement',
  project_id: c?.project_id ?? props.initial?.project ?? '',
  phase_id: c?.phase_id ?? props.initial?.phase ?? '',
  parent_id: c?.parent_id ?? props.initial?.parent ?? '',
  title: c?.title ?? '',
  justification: c?.justification ?? '',
  currency: c?.currency ?? currentMembership()?.currency ?? 'CNY',
  payee: c?.payee ?? '',
  destination: c?.destination ?? '',
  depart_on: c?.depart_on ?? '',
  return_on: c?.return_on ?? '',
  travellers: [...(c?.travellers ?? [])] as string[],
})
type Row = { spent_on: string; category: string; description: string; amount: string }
const rows = ref<Row[]>(
  (props.lines ?? []).map((l) => ({ spent_on: l.spent_on ?? '', category: l.category, description: l.description, amount: String(toMajor(l.amount_minor, f.currency)) })),
)
if (!rows.value.length) rows.value.push({ spent_on: '', category: f.kind === 'trip' ? 'flight' : 'other', description: '', amount: '' })
const total = computed(() => rows.value.reduce((s, r) => s + (r.amount ? toMinor(r.amount, f.currency) : 0), 0))
const isTrip = computed(() => f.kind === 'trip')

onMounted(async () => {
  projects.value = await api.get<Project[]>('/projects').catch(() => [])
  await loadPhases()
})
async function loadPhases() {
  phases.value = []
  if (!f.project_id) return
  const d = await api.get<{ items: Item[]; project: Project }>(`/projects/${f.project_id}`).catch(() => null)
  phases.value = d?.items.filter((i) => i.kind === 'phase') ?? []
  if (d && !c) f.currency = d.project.currency
  if (f.phase_id && !phases.value.some((p) => p.id === f.phase_id)) f.phase_id = ''
}
watch(() => f.project_id, loadPhases)

function toggleTraveller(id: string, on: boolean) {
  f.travellers = on ? [...new Set([...f.travellers, id])] : f.travellers.filter((x) => x !== id)
}
async function save(submit: boolean) {
  error.value = ''
  busy.value = true
  try {
    const body = {
      ...f,
      project_id: f.project_id || null, phase_id: f.phase_id || null, parent_id: f.parent_id || null,
      depart_on: f.depart_on || null, return_on: f.return_on || null,
      lines: rows.value.filter((r) => r.amount !== '').map((r) => ({ spent_on: r.spent_on || null, category: r.category, description: r.description, amount_minor: toMinor(r.amount, f.currency) })),
      submit: false,
    }
    const saved = c ? await api.put<Claim>(`/claims/${c.id}`, body) : await api.post<Claim>('/claims', body)
    if (files.value.length) await uploadDocs(`claims/${saved.id}`, isTrip.value ? 'ticket' : 'receipt', files.value)
    if (submit) await api.post(`/claims/${saved.id}/submit`)
    emit('saved', saved.id)
  } catch (e) {
    error.value = e instanceof Error && !('status' in e) ? t('common.error', { msg: e.message }) : errorText(e, t)
  } finally {
    busy.value = false
  }
}
const active = computed(() => props.members.filter((m) => m.status === 'active'))
</script>

<template>
  <form class="card stack" @submit.prevent="save(true)">
    <h2>{{ c ? `${c.key} ${t('lc.edit')}` : t('lc.newRequest') }}</h2>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <div class="row">
      <label class="field">{{ t('lc.kind') }}
        <select v-model="f.kind" :disabled="!!c"><option v-for="k in CLAIM_KINDS" :key="k" :value="k">{{ t(`lc.k_${k}`) }}</option></select>
      </label>
      <label class="field grow">{{ t('lc.project') }}
        <select v-model="f.project_id"><option value="">{{ t('lc.noProject') }}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{ p.key }} {{ p.name }}</option></select>
      </label>
      <label class="field grow">{{ t('lc.phase') }}
        <select v-model="f.phase_id" :disabled="!phases.length"><option value="">—</option><option v-for="p in phases" :key="p.id" :value="p.id">{{ p.title }}<template v-if="p.holder_name"> · {{ p.holder_name }}</template></option></select>
      </label>
    </div>
    <p class="muted small">{{ t(`lc.hint_${f.kind}`) }} {{ t('lc.routeHint') }}</p>
    <label class="field">{{ t('lc.title') }}<input v-model="f.title" required maxlength="300" :placeholder="t(`lc.titlePh_${f.kind}`)" /></label>
    <label class="field">{{ t('lc.justification') }}<textarea v-model="f.justification" rows="3" required maxlength="5000" :placeholder="t('lc.justificationPh')" /></label>
    <div class="row">
      <label class="field" style="flex: 0 1 110px">{{ t('auth.currency') }}<input v-model="f.currency" required maxlength="3" pattern="[A-Z]{3}" @input="f.currency = f.currency.toUpperCase()" /></label>
      <label v-if="f.kind === 'payment' || f.kind === 'purchase' || f.kind === 'advance'" class="field grow">{{ t('lc.payee') }}<input v-model="f.payee" maxlength="300" /></label>
    </div>
    <fieldset v-if="isTrip" class="stack box">
      <legend>{{ t('lc.tripDetails') }}</legend>
      <div class="row">
        <label class="field grow">{{ t('lc.destination') }}<input v-model="f.destination" required maxlength="300" /></label>
        <label class="field">{{ t('lc.depart') }}<input v-model="f.depart_on" type="date" required /></label>
        <label class="field">{{ t('lc.return_on') }}<input v-model="f.return_on" type="date" required :min="f.depart_on" /></label>
      </div>
      <div class="field">{{ t('lc.travellers') }}
        <div class="row chips">
          <label v-for="m in active" :key="m.user_id" class="chip"><input type="checkbox" :checked="f.travellers.includes(m.user_id)" @change="toggleTraveller(m.user_id, ($event.target as HTMLInputElement).checked)" /> {{ m.display_name }}</label>
        </div>
      </div>
    </fieldset>
    <div class="stack">
      <strong>{{ isTrip ? t('lc.estimate') : t('lc.lines') }}</strong>
      <div class="table-wrap">
        <table class="lines">
          <thead><tr><th>{{ t('flow.date') }}</th><th>{{ t('lc.category') }}</th><th>{{ t('work.description') }}</th><th class="num">{{ t('lc.amount') }}</th><th /></tr></thead>
          <tbody>
            <tr v-for="(r, i) in rows" :key="i">
              <td><input v-model="r.spent_on" type="date" :aria-label="t('flow.date')" /></td>
              <td><select v-model="r.category" :aria-label="t('lc.category')"><option v-for="k in LINE_CATEGORIES" :key="k" :value="k">{{ t(`lc.c_${k}`) }}</option></select></td>
              <td><input v-model="r.description" maxlength="2000" :aria-label="t('work.description')" /></td>
              <td><input v-model="r.amount" type="number" min="0" step="0.01" class="num" :aria-label="t('lc.amount')" /></td>
              <td><button type="button" :aria-label="t('work.delete')" @click="rows.splice(i, 1)">×</button></td>
            </tr>
          </tbody>
          <tfoot><tr><td colspan="3"><button type="button" @click="rows.push({ spent_on: '', category: 'other', description: '', amount: '' })">+ {{ t('lc.addLine') }}</button></td><td class="num"><strong>{{ money(total, f.currency, locale) }}</strong></td><td /></tr></tfoot>
        </table>
      </div>
    </div>
    <label class="field">{{ isTrip ? t('lc.attachTrip') : t('lc.attachReceipts') }}
      <input type="file" multiple @change="files = Array.from(($event.target as HTMLInputElement).files ?? [])" />
    </label>
    <div class="row">
      <button class="primary" type="submit" :disabled="busy">{{ t('lc.submit') }}</button>
      <button type="button" :disabled="busy" @click="save(false)">{{ t('lc.saveDraft') }}</button>
      <button type="button" @click="emit('cancel')">{{ t('common.cancel') }}</button>
    </div>
  </form>
</template>

<style scoped>
.grow { flex: 1 1 200px; }
textarea { padding: 8px 12px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); font: inherit; resize: vertical; }
.box { border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; margin: 0; }
legend { padding: 0 6px; color: var(--text-3); font-size: 13px; }
.chips { gap: 6px 14px; }
.chip { display: inline-flex; align-items: center; gap: 4px; color: var(--text); }
.chip input { height: auto; }
.lines input, .lines select { width: 100%; }
.lines td { padding: 4px; }
.num { text-align: right; }
.lines td:first-child { min-width: 140px; }
.lines td:nth-child(2) { min-width: 120px; }
.lines td:nth-child(4) { min-width: 110px; }
</style>
