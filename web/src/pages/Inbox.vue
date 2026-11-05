<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import Shell from './Shell.vue'
import { api, type Handoff, type Notification } from '../lib/api'
import { errorText } from '../lib/errors'
import { unread } from '../lib/live'
import { money } from '../lib/money'
import type { Claim, Contract } from '../lib/lifecycle'

const { t, locale } = useI18n()
const router = useRouter()
const toApprove = ref<Handoff[]>([])
const toAccept = ref<Handoff[]>([])
const notes = ref<Notification[]>([])
const error = ref('')
const notice = ref('')
const reason = reactive<Record<string, string>>({})
const fmt = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))

const money_ = ref<{ claims: Claim[]; contracts: Contract[]; to_pay: Claim[]; to_confirm: Claim[] }>({ claims: [], contracts: [], to_pay: [], to_confirm: [] })
const moneyCount = computed(() => money_.value.claims.length + money_.value.contracts.length + money_.value.to_pay.length + money_.value.to_confirm.length)
async function load() {
  money_.value = await api.get<typeof money_.value>('/approvals/waiting').catch(() => money_.value)
  const d = await api.get<{ to_approve: Handoff[]; to_accept: Handoff[]; notifications: Notification[]; unread: number }>('/inbox')
  toApprove.value = d.to_approve
  toAccept.value = d.to_accept
  notes.value = d.notifications
  unread.value = d.unread
}
onMounted(() => load().catch((e) => (error.value = errorText(e, t))))

async function decide(h: Handoff, action: string) {
  error.value = ''
  notice.value = ''
  try {
    await api.post(`/handoffs/${h.id}/decide`, { action, note: reason[h.id] ?? '' })
    notice.value = t(`flow.done_${action}`)
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
async function openNote(n: Notification) {
  if (!n.read_at) await api.post('/notifications/read', { id: n.id }).catch(() => undefined)
  if (n.link) router.push(n.link)
  else await load()
}
async function readAll() {
  await api.post('/notifications/read', {})
  await load()
}
const itemLink = (h: Handoff) => (h.project_id ? `/projects/${h.project_id}?item=${h.work_item_id}` : `/my?item=${h.work_item_id}`)
</script>

<template>
  <Shell>
    <div class="stack">
      <h1>{{ t('nav.inbox') }}</h1>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>

      <section class="stack">
        <h2>{{ t('lc.inboxTitle') }} <span class="muted">({{ moneyCount }})</span></h2>
        <p v-if="!moneyCount" class="empty">{{ t('flow.nothing') }}</p>
        <ul v-else class="notes">
          <li v-for="c in money_.claims" :key="c.id"><RouterLink class="note" :to="`/requests?id=${c.id}`"><span class="pill level-warn">{{ t('lc.toApprove') }}</span><span class="subject">{{ c.key }} {{ c.title }} <span class="muted small">— {{ c.applicant_name }} · {{ money(c.amount_minor, c.currency, locale) }}<template v-if="c.phase_title"> · {{ c.phase_title }}</template></span></span></RouterLink></li>
          <li v-for="c in money_.contracts" :key="c.id"><RouterLink class="note" :to="`/contracts?id=${c.id}`"><span class="pill level-warn">{{ t('lc.contractToApprove') }}</span><span class="subject">{{ c.key }} {{ c.title }} <span class="muted small">— {{ c.counterparty }} · {{ money(c.amount_minor, c.currency, locale) }}</span></span></RouterLink></li>
          <li v-for="c in money_.to_pay" :key="c.id"><RouterLink class="note" :to="`/requests?id=${c.id}&view=to_pay`"><span class="pill level-info">{{ t('lc.toPay') }}</span><span class="subject">{{ c.key }} {{ c.title }} <span class="muted small">— {{ c.applicant_name }} · {{ money(c.amount_minor, c.currency, locale) }}</span></span></RouterLink></li>
          <li v-for="c in money_.to_confirm" :key="c.id"><RouterLink class="note" :to="`/requests?id=${c.id}`"><span class="pill level-info">{{ t('lc.toConfirm') }}</span><span class="subject">{{ c.key }} {{ c.title }} <span class="muted small">— {{ money(c.amount_minor, c.currency, locale) }}</span></span></RouterLink></li>
        </ul>
      </section>

      <section class="stack">
        <h2>{{ t('flow.toApprove') }} <span class="muted">({{ toApprove.length }})</span></h2>
        <p v-if="!toApprove.length" class="empty">{{ t('flow.nothing') }}</p>
        <article v-for="h in toApprove" :key="h.id" class="card stack">
          <div class="row" style="justify-content: space-between">
            <RouterLink :to="itemLink(h)"><strong>{{ h.item_key }} {{ h.item_title }}</strong></RouterLink>
            <span class="muted small">{{ fmt.format(new Date(h.created_at)) }}</span>
          </div>
          <div>{{ h.from_name }} → <strong>{{ h.to_name }}</strong> · {{ h.reason }}<template v-if="h.new_due"> · {{ t('flow.newDue') }} {{ h.new_due }}</template></div>
          <p class="muted small">{{ t('flow.crossDept') }}</p>
          <div class="row">
            <button class="primary" type="button" @click="decide(h, 'approve')">{{ t('flow.approve') }}</button>
            <input v-model="reason[h.id]" :placeholder="t('flow.rejectReason')" :aria-label="t('flow.rejectReason')" style="flex: 1 1 200px" />
            <button type="button" :disabled="!(reason[h.id] ?? '').trim()" @click="decide(h, 'reject')">{{ t('flow.reject') }}</button>
          </div>
        </article>
      </section>

      <section class="stack">
        <h2>{{ t('flow.toAccept') }} <span class="muted">({{ toAccept.length }})</span></h2>
        <p v-if="!toAccept.length" class="empty">{{ t('flow.nothing') }}</p>
        <article v-for="h in toAccept" :key="h.id" class="card stack">
          <div class="row" style="justify-content: space-between">
            <RouterLink :to="itemLink(h)"><strong>{{ h.item_key }} {{ h.item_title }}</strong></RouterLink>
            <span class="muted small">{{ fmt.format(new Date(h.created_at)) }}</span>
          </div>
          <div>{{ t('flow.from') }} {{ h.from_name }} · {{ h.reason }}<template v-if="h.note"> · {{ h.note }}</template><template v-if="h.new_due"> · {{ t('flow.newDue') }} {{ h.new_due }}</template></div>
          <div class="row">
            <button class="primary" type="button" @click="decide(h, 'accept')">{{ t('flow.accept') }}</button>
            <input v-model="reason[h.id]" :placeholder="t('flow.declineReason')" :aria-label="t('flow.declineReason')" style="flex: 1 1 200px" />
            <button type="button" :disabled="!(reason[h.id] ?? '').trim()" @click="decide(h, 'decline')">{{ t('flow.decline') }}</button>
          </div>
        </article>
      </section>

      <section class="stack">
        <div class="row" style="justify-content: space-between">
          <h2>{{ t('flow.notifications') }}</h2>
          <button v-if="notes.some((n) => !n.read_at)" type="button" @click="readAll">{{ t('flow.readAll') }}</button>
        </div>
        <p v-if="!notes.length" class="empty">{{ t('flow.nothing') }}</p>
        <ul class="notes">
          <li v-for="n in notes" :key="n.id">
            <button type="button" class="note" :class="{ unread: !n.read_at }" @click="openNote(n)">
              <span class="pill" :class="n.kind === 'alert' ? 'level-critical' : n.kind.startsWith('handoff') || n.kind === 'approval_request' ? 'level-warn' : 'level-info'">{{ t(`flow.n_${n.kind}`) }}</span>
              <span class="subject">{{ n.subject }}<span v-if="n.body" class="muted small"> — {{ n.body }}</span></span>
              <span class="muted small">{{ fmt.format(new Date(n.created_at)) }}</span>
            </button>
          </li>
        </ul>
      </section>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
.notes { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.note { text-decoration: none; box-sizing: border-box; width: 100%; height: auto; display: flex; flex-wrap: wrap; gap: 10px; align-items: center; text-align: left; padding: 10px 14px; background: var(--surface); border: 1px solid var(--line); border-radius: 10px; color: var(--text); }
.note.unread { border-left: 4px solid var(--accent); font-weight: 500; }
.subject { flex: 1 1 260px; min-width: 0; overflow-wrap: anywhere; }
</style>
