<script setup lang="ts">
// One request: what, why, how much, files, the approval route and history, and the next step.
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Item, type Member } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { money } from '../../lib/money'
import { me, currentMembership } from '../../lib/session'
import { STATUS_TONE, uploadDocs, type Action, type ApprovalView, type Claim, type Doc, type Line } from '../../lib/lifecycle'
import ApprovalTrail from './ApprovalTrail.vue'
import RequestForm from './RequestForm.vue'

const props = defineProps<{ id: string; members: Member[] }>()
const emit = defineEmits<{ close: []; changed: []; open: [id: string]; fileFor: [trip: Claim] }>()
const { t, locale } = useI18n()
type Detail = { claim: Claim; lines: Line[]; documents: Doc[]; actions: Action[]; approval: ApprovalView | null; children: Claim[]; bookings: Item[]; travellers: { id: string; name: string }[] }
const d = ref<Detail | null>(null)
const error = ref('')
const notice = ref('')
const editing = ref(false)
const payRef = ref('')
const booking = ref({ to_user: '', title: '', due_on: '' })
const myId = computed(() => me.value?.user_id ?? '')
const money_ = computed(() => !!(currentMembership()?.is_admin || currentMembership()?.is_finance))

async function load() {
  error.value = ''
  try {
    d.value = await api.get<Detail>(`/claims/${props.id}`)
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
const c = computed(() => d.value?.claim)
const mine = computed(() => c.value?.applicant_id === myId.value)
const myTurn = computed(() => !!c.value && c.value.status === 'in_approval' && c.value.waiting_for.includes(myId.value))
const canBook = computed(() => !!c.value && c.value.kind === 'trip' && c.value.status === 'approved' && (mine.value || c.value.travellers.includes(myId.value) || money_.value))
const decide = (b: object) => run(() => api.post(`/claims/${props.id}/decide`, b), t('lc.decided'))
const total = computed(() => (c.value ? money(c.value.amount_minor, c.value.currency, locale.value) : ''))
const docLabel = ref('receipt')
const fmtDate = (s: string | null) => s ?? '—'
</script>

<template>
  <aside class="card stack panel" :aria-label="c?.key ?? ''">
    <div class="row head">
      <div style="min-width: 0">
        <div class="muted small">{{ c?.key }} · {{ c ? t(`lc.k_${c.kind}`) : '' }}</div>
        <h2>{{ c?.title }}</h2>
      </div>
      <button type="button" :aria-label="t('work.close')" @click="emit('close')">×</button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
    <template v-if="d && c">
      <RequestForm v-if="editing" :members="members" :claim="c" :lines="d.lines" @saved="editing = false; run(async () => undefined)" @cancel="editing = false" />
      <template v-else>
        <div class="row">
          <span class="pill" :class="STATUS_TONE[c.status]">{{ t(`lc.s_${c.status}`) }}</span>
          <strong class="amount">{{ total }}</strong>
          <span v-if="c.project_currency && c.project_currency !== c.currency && c.project_amount_minor !== null" class="muted small">≈ {{ money(c.project_amount_minor, c.project_currency, locale) }}</span>
        </div>
        <dl class="facts">
          <dt>{{ t('lc.applicant') }}</dt><dd>{{ c.applicant_name }}</dd>
          <dt>{{ t('lc.project') }}</dt><dd><RouterLink v-if="c.project_id" :to="`/projects/${c.project_id}?tab=file`">{{ c.project_key }} {{ c.project_name }}</RouterLink><template v-else>{{ t('lc.noProject') }}</template></dd>
          <dt>{{ t('lc.phase') }}</dt><dd>{{ c.phase_title ?? '—' }}</dd>
          <template v-if="c.parent_id"><dt>{{ t('lc.forTrip') }}</dt><dd><a href="#" @click.prevent="emit('open', c.parent_id!)">{{ c.parent_key }}</a></dd></template>
          <template v-if="c.payee"><dt>{{ t('lc.payee') }}</dt><dd>{{ c.payee }}</dd></template>
          <template v-if="c.kind === 'trip'">
            <dt>{{ t('lc.destination') }}</dt><dd>{{ c.destination }}</dd>
            <dt>{{ t('lc.dates') }}</dt><dd>{{ fmtDate(c.depart_on) }} → {{ fmtDate(c.return_on) }}</dd>
            <dt>{{ t('lc.travellers') }}</dt><dd>{{ d.travellers.map((x) => x.name).join('、') || '—' }}</dd>
          </template>
          <template v-if="c.paid_at"><dt>{{ t('lc.paidRef') }}</dt><dd>{{ c.payment_ref || '—' }} · {{ new Date(c.paid_at).toLocaleDateString(locale) }}</dd></template>
        </dl>
        <div><div class="muted small">{{ t('lc.justification') }}</div><p class="why">{{ c.justification || '—' }}</p></div>

        <div v-if="d.lines.length" class="table-wrap">
          <table>
            <thead><tr><th>{{ t('flow.date') }}</th><th>{{ t('lc.category') }}</th><th>{{ t('work.description') }}</th><th class="num">{{ t('lc.amount') }}</th></tr></thead>
            <tbody><tr v-for="l in d.lines" :key="l.id"><td>{{ l.spent_on ?? '' }}</td><td>{{ t(`lc.c_${l.category}`) }}</td><td>{{ l.description }}</td><td class="num">{{ money(l.amount_minor, c.currency, locale) }}</td></tr></tbody>
          </table>
        </div>

        <section class="stack">
          <strong>{{ t('lc.files') }}</strong>
          <ul v-if="d.documents.length" class="files">
            <li v-for="doc in d.documents" :key="doc.id"><a :href="`/api/v1/documents/${doc.id}`" target="_blank" rel="noopener">{{ doc.file_name }}</a> <span class="muted small">{{ doc.label ? t(`lc.l_${doc.label}`, doc.label) : '' }}<template v-if="doc.version > 1"> v{{ doc.version }}</template> · {{ doc.uploaded_by_name }}</span></li>
          </ul>
          <p v-else class="empty">{{ t('lc.noFiles') }}</p>
          <div v-if="mine || c.travellers.includes(myId) || money_" class="row">
            <select v-model="docLabel" :aria-label="t('lc.fileKind')"><option value="receipt">{{ t('lc.l_receipt') }}</option><option value="invoice">{{ t('lc.l_invoice') }}</option><option value="ticket">{{ t('lc.l_ticket') }}</option><option value="quote">{{ t('lc.l_quote') }}</option><option value="payment">{{ t('lc.l_payment') }}</option></select>
            <input type="file" multiple :aria-label="t('lc.files')" @change="(e) => { const fs = (e.target as HTMLInputElement).files; if (fs?.length) run(() => uploadDocs(`claims/${c!.id}`, docLabel, fs), t('work.saved')) }" />
          </div>
        </section>

        <section class="stack">
          <strong>{{ t('lc.approvalTitle') }}</strong>
          <ApprovalTrail :approval="d.approval" :actions="d.actions" :my-turn="myTurn" :members="members" :me="myId" @decide="decide" />
        </section>

        <section v-if="c.kind === 'trip'" class="stack">
          <strong>{{ t('lc.bookings') }}</strong>
          <ul v-if="d.bookings.length" class="files">
            <li v-for="b in d.bookings" :key="b.id"><RouterLink :to="`/my?item=${b.id}`">{{ b.key }} {{ b.title }}</RouterLink> <span class="muted small">{{ b.holder_name }} · {{ t(`work.s_${b.status}`) }}</span></li>
          </ul>
          <p v-else class="empty">{{ t('lc.noBookings') }}</p>
          <form v-if="canBook" class="row" @submit.prevent="run(() => api.post(`/claims/${c!.id}/bookings`, { ...booking, due_on: booking.due_on || null }), t('lc.bookingSent')).then(() => (booking = { to_user: '', title: '', due_on: '' }))">
            <label class="field grow">{{ t('lc.bookingWhat') }}<input v-model="booking.title" required maxlength="300" :placeholder="t('lc.bookingPh')" /></label>
            <label class="field">{{ t('lc.bookingWho') }}<select v-model="booking.to_user" required><option value="" disabled>{{ t('org.choose') }}</option><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select></label>
            <label class="field">{{ t('flow.due') }}<input v-model="booking.due_on" type="date" /></label>
            <button class="primary" type="submit" style="align-self: flex-end">{{ t('lc.delegateBooking') }}</button>
          </form>
          <strong>{{ t('lc.tripCosts') }}</strong>
          <ul v-if="d.children.length" class="files">
            <li v-for="x in d.children" :key="x.id"><a href="#" @click.prevent="emit('open', x.id)">{{ x.key }} {{ x.title }}</a> <span class="muted small">{{ money(x.amount_minor, x.currency, locale) }} · {{ t(`lc.s_${x.status}`) }}</span></li>
          </ul>
          <button v-if="c.status === 'approved' || c.status === 'closed'" type="button" @click="emit('fileFor', c)">+ {{ t('lc.fileTripCost') }}</button>
        </section>

        <div class="row actions">
          <button v-if="mine && (c.status === 'draft' || c.status === 'returned')" type="button" @click="editing = true">{{ t('lc.edit') }}</button>
          <button v-if="mine && (c.status === 'draft' || c.status === 'returned')" class="primary" type="button" @click="run(() => api.post(`/claims/${c!.id}/submit`), t('lc.submitted'))">{{ t('lc.submit') }}</button>
          <button v-if="mine && ['draft', 'returned', 'in_approval'].includes(c.status)" type="button" @click="run(() => api.post(`/claims/${c!.id}/cancel`, {}), t('lc.s_cancelled'))">{{ t('lc.withdraw') }}</button>
          <template v-if="money_ && c.status === 'approved' && c.kind !== 'trip'">
            <input v-model="payRef" :placeholder="t('lc.payRefPh')" :aria-label="t('lc.paidRef')" maxlength="200" />
            <button class="primary" type="button" @click="run(() => api.post(`/claims/${c!.id}/pay`, { reference: payRef }), t('lc.paidDone'))">{{ t('lc.pay') }}</button>
          </template>
          <button v-if="mine && c.status === 'paid'" class="primary" type="button" @click="run(() => api.post(`/claims/${c!.id}/close`, {}), t('lc.s_closed'))">{{ t('lc.confirmReceipt') }}</button>
          <button v-if="mine && c.kind === 'trip' && c.status === 'approved'" type="button" @click="run(() => api.post(`/claims/${c!.id}/close`, {}), t('lc.s_closed'))">{{ t('lc.closeTrip') }}</button>
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
.why { margin: 4px 0 0; white-space: pre-wrap; }
.files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.num { text-align: right; }
.grow { flex: 1 1 180px; }
.actions { border-top: 1px solid var(--line); padding-top: 12px; }
@media (max-width: 1000px) { .panel { position: static; max-height: none; } }
</style>
