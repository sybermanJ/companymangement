<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Item, type Member } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { money, toMinor } from '../../lib/money'
import { me, currentMembership } from '../../lib/session'
import { STATUS_TONE, uploadDocs, type Action, type ApprovalView, type Contract, type Doc, type Milestone } from '../../lib/lifecycle'
import ApprovalTrail from './ApprovalTrail.vue'
import ContractForm from './ContractForm.vue'

const props = defineProps<{ id: string; members: Member[] }>()
const emit = defineEmits<{ close: []; changed: [] }>()
const { t, locale } = useI18n()
type Detail = { contract: Contract; milestones: Milestone[]; documents: Doc[]; actions: Action[]; approval: ApprovalView | null }
const d = ref<Detail | null>(null)
const phases = ref<Item[]>([])
const error = ref('')
const notice = ref('')
const editing = ref(false)
const myId = computed(() => me.value?.user_id ?? '')
const sees = computed(() => !!(currentMembership()?.is_admin || currentMembership()?.is_finance))

async function load() {
  error.value = ''
  try {
    d.value = await api.get<Detail>(`/contracts/${props.id}`)
    const p = d.value.contract.project_id
    phases.value = p ? ((await api.get<{ items: Item[] }>(`/projects/${p}`).catch(() => null))?.items.filter((i) => i.kind === 'phase') ?? []) : []
  } catch (e) {
    error.value = errorText(e, t)
  }
}
watch(() => props.id, () => { editing.value = false; notice.value = ''; load() }, { immediate: true })
async function run(fn: () => Promise<unknown>, ok?: string) {
  error.value = ''
  notice.value = ''
  try {
    await fn()
    if (ok) notice.value = ok
    await load()
    emit('changed')
  } catch (e) {
    error.value = e instanceof Error && !('status' in e) ? t('common.error', { msg: e.message }) : errorText(e, t)
  }
}
const c = computed(() => d.value?.contract)
const runs = computed(() => !!c.value && (c.value.owner_id === myId.value || c.value.created_by === myId.value || sees.value))
const myTurn = computed(() => !!c.value && c.value.status === 'in_approval' && c.value.waiting_for.includes(myId.value))
const signedOn = ref(new Date().toISOString().slice(0, 10))
const ms = reactive({ title: '', phase_id: '', due_on: '', amount: '' })
const docLabel = ref('signed')
const pct = computed(() => (c.value && c.value.amount_minor > 0 ? Math.round((c.value.settled_minor * 100) / c.value.amount_minor) : null))
function addMilestone() {
  const cur = c.value!
  run(() => api.post(`/contracts/${cur.id}/milestones`, { title: ms.title, phase_id: ms.phase_id || null, due_on: ms.due_on || null, amount_minor: ms.amount ? toMinor(ms.amount, cur.currency) : 0 }), t('work.saved'))
    .then(() => Object.assign(ms, { title: '', phase_id: '', due_on: '', amount: '' }))
}
</script>

<template>
  <aside class="card stack panel">
    <div class="row head">
      <div style="min-width: 0">
        <div class="muted small">{{ c?.key }} · {{ c ? t(`lc.ck_${c.kind}`) : '' }}</div>
        <h2>{{ c?.title }}</h2>
      </div>
      <button type="button" :aria-label="t('work.close')" @click="emit('close')">×</button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
    <template v-if="d && c">
      <ContractForm v-if="editing" :members="members" :contract="c" @saved="editing = false; run(async () => undefined)" @cancel="editing = false" />
      <template v-else>
        <div class="row">
          <span class="pill" :class="STATUS_TONE[c.status]">{{ t(`lc.s_${c.status}`) }}</span>
          <strong class="amount">{{ money(c.amount_minor, c.currency, locale) }}</strong>
          <span v-if="pct !== null" class="muted small">{{ t('lc.settledPct', { pct, amount: money(c.settled_minor, c.currency, locale) }) }}</span>
        </div>
        <dl class="facts">
          <dt>{{ t('lc.counterparty') }}</dt><dd>{{ c.counterparty || '—' }}</dd>
          <dt>{{ t('lc.project') }}</dt><dd><RouterLink v-if="c.project_id" :to="`/projects/${c.project_id}?tab=file`">{{ c.project_key }} {{ c.project_name }}</RouterLink><template v-else>—</template></dd>
          <dt>{{ t('lc.owner') }}</dt><dd>{{ c.owner_name }}</dd>
          <dt>{{ t('lc.term') }}</dt><dd>{{ c.starts_on ?? '—' }} → {{ c.ends_on ?? '—' }}</dd>
          <template v-if="c.signed_on"><dt>{{ t('lc.signedOn') }}</dt><dd>{{ c.signed_on }}</dd></template>
        </dl>
        <p v-if="c.summary" class="why">{{ c.summary }}</p>

        <section class="stack">
          <strong>{{ t('lc.files') }}</strong>
          <ul v-if="d.documents.length" class="files">
            <li v-for="doc in d.documents" :key="doc.id"><a :href="`/api/v1/documents/${doc.id}`" target="_blank" rel="noopener">{{ doc.file_name }}</a> <span class="muted small">{{ doc.label ? t(`lc.l_${doc.label}`, doc.label) : '' }} v{{ doc.version }} · {{ doc.uploaded_by_name }} · {{ new Date(doc.created_at).toLocaleDateString(locale) }}</span></li>
          </ul>
          <p v-else class="empty">{{ t('lc.noFiles') }}</p>
          <div v-if="runs" class="row">
            <select v-model="docLabel" :aria-label="t('lc.fileKind')"><option value="draft">{{ t('lc.l_draft') }}</option><option value="signed">{{ t('lc.l_signed') }}</option><option value="amendment">{{ t('lc.l_amendment') }}</option><option value="invoice">{{ t('lc.l_invoice') }}</option><option value="acceptance">{{ t('lc.l_acceptance') }}</option></select>
            <input type="file" multiple :aria-label="t('lc.files')" @change="(e) => { const fs = (e.target as HTMLInputElement).files; if (fs?.length) run(() => uploadDocs(`contracts/${c!.id}`, docLabel, fs), t('work.saved')) }" />
          </div>
          <p class="muted small">{{ t('lc.versionHint') }}</p>
        </section>

        <section class="stack">
          <strong>{{ t('lc.milestones') }}</strong>
          <div v-if="d.milestones.length" class="table-wrap">
            <table>
              <thead><tr><th>{{ t('lc.milestone') }}</th><th>{{ t('lc.phase') }}</th><th>{{ t('flow.due') }}</th><th class="num">{{ t('lc.amount') }}</th><th>{{ t('org.status') }}</th></tr></thead>
              <tbody>
                <tr v-for="m in d.milestones" :key="m.id">
                  <td>{{ m.title }}</td><td>{{ m.phase_title ?? '—' }}</td><td>{{ m.due_on ?? '—' }}</td><td class="num">{{ money(m.amount_minor, c.currency, locale) }}</td>
                  <td>
                    <select v-if="runs" :value="m.status" :aria-label="t('org.status')" @change="run(() => api.post(`/milestones/${m.id}/status`, { status: ($event.target as HTMLSelectElement).value }), t('work.saved'))">
                      <option v-for="s in ['planned', 'invoiced', 'settled']" :key="s" :value="s">{{ t(`lc.m_${s}`) }}</option>
                    </select>
                    <span v-else>{{ t(`lc.m_${m.status}`) }}</span>
                    <div v-if="m.settled_on" class="muted small">{{ m.settled_on }}</div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <form v-if="runs" class="row" @submit.prevent="addMilestone">
            <label class="field grow">{{ t('lc.milestone') }}<input v-model="ms.title" required maxlength="300" :placeholder="t('lc.milestonePh')" /></label>
            <label class="field">{{ t('lc.phase') }}<select v-model="ms.phase_id"><option value="">—</option><option v-for="p in phases" :key="p.id" :value="p.id">{{ p.title }}</option></select></label>
            <label class="field">{{ t('flow.due') }}<input v-model="ms.due_on" type="date" /></label>
            <label class="field" style="flex: 0 1 130px">{{ t('lc.amount') }}<input v-model="ms.amount" type="number" min="0" step="0.01" /></label>
            <button type="submit" style="align-self: flex-end">+ {{ t('lc.addMilestone') }}</button>
          </form>
        </section>

        <section class="stack">
          <strong>{{ t('lc.approvalTitle') }}</strong>
          <ApprovalTrail :approval="d.approval" :actions="d.actions" :my-turn="myTurn" :members="members" :me="myId" @decide="(b) => run(() => api.post(`/contracts/${c!.id}/decide`, b), t('lc.decided'))" />
        </section>

        <div v-if="runs" class="row actions">
          <button v-if="c.status === 'draft' || c.status === 'returned'" type="button" @click="editing = true">{{ t('lc.edit') }}</button>
          <button v-if="c.status === 'draft' || c.status === 'returned'" class="primary" type="button" @click="run(() => api.post(`/contracts/${c!.id}/submit`), t('lc.submitted'))">{{ t('lc.submitContract') }}</button>
          <template v-if="c.status === 'approved'">
            <label class="field">{{ t('lc.signedOn') }}<input v-model="signedOn" type="date" /></label>
            <button class="primary" type="button" style="align-self: flex-end" @click="run(() => api.post(`/contracts/${c!.id}/status`, { status: 'signed', signed_on: signedOn }), t('lc.s_signed'))">{{ t('lc.markSigned') }}</button>
          </template>
          <template v-if="c.status === 'signed'">
            <button type="button" @click="run(() => api.post(`/contracts/${c!.id}/status`, { status: 'completed' }), t('lc.s_completed'))">{{ t('lc.markCompleted') }}</button>
            <button type="button" @click="run(() => api.post(`/contracts/${c!.id}/status`, { status: 'terminated' }), t('lc.s_terminated'))">{{ t('lc.markTerminated') }}</button>
          </template>
        </div>
      </template>
    </template>
  </aside>
</template>

<style scoped>
.panel { position: sticky; top: 12px; max-height: calc(100vh - 24px); overflow-y: auto; }
.head { justify-content: space-between; align-items: flex-start; flex-wrap: nowrap; }
.amount { font-size: 20px; }
.facts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 14px; margin: 0; }
.facts dt { color: var(--text-3); }
.facts dd { margin: 0; overflow-wrap: anywhere; }
.why { margin: 0; white-space: pre-wrap; }
.files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.num { text-align: right; white-space: nowrap; }
.grow { flex: 1 1 180px; }
.actions { border-top: 1px solid var(--line); padding-top: 12px; align-items: flex-end; }
@media (max-width: 1000px) { .panel { position: static; max-height: none; } }
</style>
