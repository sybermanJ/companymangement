<script setup lang="ts">
// Side panel for one work item: status, dates, progress, logging with files, timer, history.
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, uploadFiles, ApiError, type Attachment, type Commit, type Handoff, type Item, type ItemDetail, type ItemEvent, type Member, type Spec, type Submission, type Timer, type WorkLog } from '../lib/api'
import { me } from '../lib/session'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'

const props = defineProps<{ itemId: string; members: Member[] }>()
const emit = defineEmits<{ close: []; changed: [] }>()
const { t, locale } = useI18n()

const item = ref<Item | null>(null)
const logs = ref<WorkLog[]>([])
const attachments = ref<Attachment[]>([])
const events = ref<ItemEvent[]>([])
const canManage = ref(false)
const specs = ref<Spec[]>([])
const submissions = ref<Submission[]>([])
const handoffs = ref<Handoff[]>([])
const commits = ref<Commit[]>([])
const canReassign = ref(false)
const canReturn = ref(false)
const timer = ref<Timer | null>(null)
const error = ref('')
const notice = ref('')
const STATUSES = ['todo', 'in_progress', 'blocked', 'in_review', 'done', 'cancelled']

const isHolder = computed(() => item.value?.holder_user_id === me.value?.user_id)
const canEditStatus = computed(() => canManage.value || isHolder.value)
const fmt = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))

async function load() {
  const d = await api.get<ItemDetail>(`/items/${props.itemId}`)
  item.value = d.item
  specs.value = d.specs
  submissions.value = d.submissions
  handoffs.value = d.handoffs
  commits.value = d.commits
  canReassign.value = d.can_reassign
  canReturn.value = d.can_return
  logs.value = d.logs
  attachments.value = d.attachments
  events.value = d.events.slice().reverse()
  canManage.value = d.can_manage
  timer.value = (await api.get<{ timer: Timer | null }>('/my/items')).timer
}
onMounted(() => load().catch((e) => (error.value = errorText(e, t))))
watch(() => props.itemId, () => load().catch((e) => (error.value = errorText(e, t))))

async function act(fn: () => Promise<unknown>, ok?: string) {
  error.value = ''
  notice.value = ''
  try {
    await fn()
    if (ok) notice.value = ok
    emit('changed')
  } catch (e) {
    error.value = e instanceof ApiError && e.status === 409 ? t('work.conflict') : errorText(e, t)
  }
  await load().catch(() => undefined)
}

const missing = computed(() => specs.value.filter((s) => s.required && s.submitted < s.min_count))
const isOpen = computed(() => !!item.value && !['done', 'cancelled'].includes(item.value.status))
const setStatus = (status: string) => act(() => api.post(`/items/${props.itemId}/status`, { version: item.value!.version, status }))
const overrideReason = ref('')
const forceDone = () => act(() => api.post(`/items/${props.itemId}/status`, { version: item.value!.version, status: 'done', override_deliverables: true, note: overrideReason.value }), t('work.saved'))

// Deliverables
const subValue = reactive<Record<string, string>>({})
const submitValue = (s: Spec) => act(async () => {
  await api.post(`/items/${props.itemId}/submissions`, { spec_id: s.id, value: subValue[s.id] ?? '' })
  subValue[s.id] = ''
}, t('work.saved'))
const submitFile = (s: Spec, e: Event) => {
  const input = e.target as HTMLInputElement
  const fs = [...(input.files ?? [])]
  if (!fs.length) return
  return act(async () => {
    await uploadFiles(props.itemId, fs, undefined, s.id)
    input.value = ''
  }, t('work.saved'))
}
const MEDIA = ['image', 'audio', 'video', 'file']
const ACCEPT: Record<string, string> = { image: 'image/*', audio: 'audio/*', video: 'video/*', file: '*/*' }
const subsFor = (id: string) => submissions.value.filter((x) => x.spec_id === id)
const attName = (id: string | null) => attachments.value.find((a) => a.id === id)?.file_name ?? ''
const newSpec = reactive({ kind: 'text', instructions: '', min_count: 1 })
const addSpec = () => act(async () => {
  await api.post(`/items/${props.itemId}/deliverables`, { ...newSpec })
  newSpec.instructions = ''
}, t('work.saved'))

// Hand-off
const ho = reactive({ to_user: '', reason: '', new_due: '' })
const others = computed(() => props.members.filter((m) => m.status === 'active' && m.user_id !== item.value?.holder_user_id))
const handOff = () => act(async () => {
  await api.post(`/items/${props.itemId}/handoffs`, { version: item.value!.version, to_user: ho.to_user, reason: ho.reason, new_due: ho.new_due || null })
  Object.assign(ho, { to_user: '', reason: '', new_due: '' })
}, t('flow.handoffSent'))
const openHandoff = computed(() => handoffs.value.find((h) => h.state === 'pending_approval' || h.state === 'pending_accept'))
const decide = (h: Handoff, action: string, note = '') => act(() => api.post(`/handoffs/${h.id}/decide`, { action, note }))
const declineNote = ref('')
const returnBack = () => act(() => api.post(`/items/${props.itemId}/return`, { reason: '' }))

// Commits
const cm = reactive({ sha: '', url: '', message: '' })
const linkCommit = () => act(async () => {
  await api.post(`/items/${props.itemId}/commits`, { ...cm })
  Object.assign(cm, { sha: '', url: '', message: '' })
}, t('work.saved'))
const patch = (body: Record<string, unknown>) => act(() => api.patch(`/items/${props.itemId}`, { version: item.value!.version, ...body }), t('work.saved'))

const log = reactive({ minutes: 60, custom: '', description: '' })
const files = ref<File[]>([])
const fileInput = ref<HTMLInputElement | null>(null)
function pickFiles(e: Event) {
  files.value = [...((e.target as HTMLInputElement).files ?? [])]
}
const saveLog = () => act(async () => {
  const minutes = log.custom ? Math.round(Number(log.custom) * 60) : log.minutes
  const row = await api.post<WorkLog>(`/items/${props.itemId}/logs`, { minutes, description: log.description })
  if (files.value.length) await uploadFiles(props.itemId, files.value, row.id)
  log.description = ''
  log.custom = ''
  files.value = []
  if (fileInput.value) fileInput.value.value = ''
}, t('work.saved'))
const deleteLog = (id: string) => act(() => api.del(`/logs/${id}`))
const startTimer = () => act(() => api.post('/timer/start', { work_item_id: props.itemId }))
const stopTimer = () => act(() => api.post('/timer/stop', { description: log.description }))

const attFor = (logId: string) => attachments.value.filter((a) => a.work_log_id === logId)
const loose = computed(() => attachments.value.filter((a) => !a.work_log_id))
const url = (a: Attachment) => `/api/v1/attachments/${a.id}`

function evText(e: ItemEvent): string {
  const p = e.payload as Record<string, unknown>
  const st = (s: unknown) => t(`work.s_${String(s)}`)
  switch (e.kind) {
    case 'status_changed': return t('work.ev_status_changed', { from: st(p.from), to: st(p.to) })
    case 'logged': case 'log_deleted': return t(`work.ev_${e.kind}`, { h: hours(Number(p.minutes)) })
    case 'attachment_added': return t('work.ev_attachment_added', { name: String(p.name) })
    case 'handoff_requested': return t('flow.ev_handoff_requested', { to: String(p.to_name ?? ''), reason: String(p.reason ?? '') })
    case 'reassigned': return t('flow.ev_reassigned', { from: String(p.from_name ?? ''), to: String(p.to_name ?? '') })
    case 'commit_linked': return t('flow.ev_commit_linked', { sha: String(p.sha ?? '') })
    case 'deliverable_submitted': return t('flow.ev_deliverable_submitted', { kind: t(`flow.k_${String(p.kind)}`) })
    case 'handoff_approved': case 'handoff_rejected': case 'handoff_declined': case 'handoff_cancelled': case 'claimed':
      return t(`flow.ev_${e.kind}`) + (p.note ? `：${String(p.note)}` : '')
    default: return t(`work.ev_${e.kind}`)
  }
}
</script>

<template>
  <aside class="panel" :aria-label="item?.title ?? ''">
    <div class="row" style="justify-content: space-between">
      <span class="muted">{{ item?.key }}</span>
      <button type="button" :aria-label="t('work.close')" @click="emit('close')">✕</button>
    </div>
    <template v-if="item">
      <h2>{{ item.title }}</h2>
      <p v-if="item.kind === 'routine' || item.kind === 'request'" class="muted small">{{ t(`flow.kind_${item.kind}`) }}<template v-if="item.occurrence_date"> · {{ item.occurrence_date }}</template><template v-if="item.handoff_count"> · {{ t('flow.handoffCount', { n: item.handoff_count }) }}</template></p>
      <div v-if="openHandoff" class="notice warn stack" role="status">
        <span>{{ t(`flow.state_${openHandoff.state}`) }}：{{ openHandoff.from_name }} → {{ openHandoff.to_name }} · {{ openHandoff.reason }}</span>
        <div v-if="openHandoff.state === 'pending_accept' && openHandoff.to_user === me?.user_id" class="row">
          <button class="primary" type="button" @click="decide(openHandoff, 'accept')">{{ t('flow.accept') }}</button>
          <input v-model="declineNote" :placeholder="t('flow.declineReason')" :aria-label="t('flow.declineReason')" />
          <button type="button" :disabled="!declineNote.trim()" @click="decide(openHandoff, 'decline', declineNote)">{{ t('flow.decline') }}</button>
        </div>
        <div v-if="openHandoff.from_user === me?.user_id || openHandoff.requested_by === me?.user_id" class="row">
          <button type="button" @click="decide(openHandoff, 'cancel')">{{ t('flow.cancelHandoff') }}</button>
        </div>
      </div>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>

      <div class="grid2">
        <label class="field">{{ t('work.status') }}
          <select :value="item.status" :disabled="!canEditStatus" @change="setStatus(($event.target as HTMLSelectElement).value)">
            <option v-for="s in STATUSES" :key="s" :value="s">{{ t(`work.s_${s}`) }}</option>
          </select>
        </label>
        <label class="field">{{ t('work.holder') }}
          <select :value="item.holder_user_id ?? ''" :disabled="!canManage" @change="patch({ holder_user_id: ($event.target as HTMLSelectElement).value || null })">
            <option value="">{{ t('work.unassigned') }}</option>
            <option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option>
          </select>
        </label>
        <label class="field">{{ t('work.start') }}<input type="date" :value="item.planned_start ?? ''" :disabled="!canManage" @change="patch({ planned_start: ($event.target as HTMLInputElement).value || null })" /></label>
        <label class="field">{{ t('work.end') }}<input type="date" :value="item.planned_end ?? ''" :disabled="!canManage" @change="patch({ planned_end: ($event.target as HTMLInputElement).value || null })" /></label>
      </div>
      <div v-if="item.kind === 'task'" class="row">
        <label class="field" style="flex: 1 1 200px">{{ t('work.progress') }} · {{ item.progress_pct }}%
          <input type="range" min="0" max="100" step="5" :value="item.progress_pct" :disabled="!canEditStatus" @change="patch({ progress_pct: Number(($event.target as HTMLInputElement).value) })" />
        </label>
        <div class="muted">{{ t('work.logged') }} {{ hours(item.logged_minutes) }} / {{ hours(item.estimate_minutes ?? 0) }} {{ t('work.hours') }}</div>
      </div>
      <p v-if="item.description" class="desc">{{ item.description }}</p>

      <section v-if="specs.length || canManage" class="stack">
        <h3>{{ t('flow.deliverables') }}</h3>
        <p v-if="!specs.length" class="muted small">{{ t('flow.noDeliverables') }}</p>
        <article v-for="s in specs" :key="s.id" class="spec" :class="{ met: s.submitted >= s.min_count }">
          <div class="row" style="justify-content: space-between">
            <strong>{{ s.submitted >= s.min_count ? '✓' : '○' }} {{ t(`flow.k_${s.kind}`) }}<template v-if="s.min_count > 1"> ×{{ s.min_count }}</template></strong>
            <span class="muted small">{{ s.submitted }}/{{ s.min_count }}<template v-if="!s.required"> · {{ t('flow.optional') }}</template></span>
          </div>
          <p v-if="s.instructions" class="muted small desc">{{ s.instructions }}</p>
          <ul v-if="subsFor(s.id).length" class="subs">
            <li v-for="x in subsFor(s.id)" :key="x.id" class="small"><template v-if="x.attachment_id"><a :href="`/api/v1/attachments/${x.attachment_id}`" target="_blank" rel="noopener">{{ attName(x.attachment_id) || t('flow.file') }}</a></template><template v-else-if="/^https?:\/\//.test(x.value)"><a :href="x.value" target="_blank" rel="noopener noreferrer">{{ x.value }}</a></template><template v-else>{{ x.value }}</template> · {{ x.submitted_by_name }}</li>
          </ul>
          <div v-if="isOpen && (isHolder || canManage)" class="row">
            <template v-if="MEDIA.includes(s.kind)">
              <label class="field" style="flex: 1 1 200px">{{ t('flow.uploadFor') }}<input type="file" :accept="ACCEPT[s.kind]" @change="submitFile(s, $event)" /></label>
            </template>
            <template v-else>
              <input v-model="subValue[s.id]" :placeholder="t(`flow.ph_${s.kind}`)" :aria-label="t(`flow.k_${s.kind}`)" style="flex: 1 1 200px" />
              <button type="button" :disabled="!(subValue[s.id] ?? '').trim()" @click="submitValue(s)">{{ t('flow.submit') }}</button>
            </template>
          </div>
        </article>
        <div v-if="missing.length && canManage && isOpen" class="row">
          <input v-model="overrideReason" :placeholder="t('flow.overrideReason')" :aria-label="t('flow.overrideReason')" style="flex: 1 1 200px" />
          <button type="button" :disabled="!overrideReason.trim()" @click="forceDone">{{ t('flow.forceDone') }}</button>
        </div>
        <form v-if="canManage && isOpen" class="row" @submit.prevent="addSpec">
          <select v-model="newSpec.kind" :aria-label="t('flow.kind')"><option v-for="k in ['text', 'image', 'audio', 'video', 'file', 'git_commit', 'link', 'checklist']" :key="k" :value="k">{{ t(`flow.k_${k}`) }}</option></select>
          <input v-model="newSpec.instructions" :placeholder="t('flow.instructions')" :aria-label="t('flow.instructions')" style="flex: 1 1 160px" />
          <button type="submit">+ {{ t('flow.addDeliverable') }}</button>
        </form>
      </section>

      <section v-if="isOpen && (canReassign || canReturn)" class="stack box">
        <h3>{{ t('flow.handOff') }}</h3>
        <form v-if="canReassign && !openHandoff" class="stack" @submit.prevent="handOff">
          <div class="grid2">
            <label class="field">{{ t('flow.to') }}
              <select v-model="ho.to_user" required><option value="">{{ t('org.choose') }}</option><option v-for="m in others" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
            </label>
            <label class="field">{{ t('flow.newDue') }}<input v-model="ho.new_due" type="date" /></label>
          </div>
          <label class="field">{{ t('flow.reason') }}<input v-model="ho.reason" required maxlength="500" /></label>
          <p class="muted small">{{ t('flow.handoffHint') }}</p>
          <div class="row"><button class="primary" type="submit">{{ t('flow.send') }}</button></div>
        </form>
        <div v-if="canReturn && !openHandoff" class="row"><button type="button" @click="returnBack">↩ {{ t('flow.returnBack') }}</button></div>
      </section>

      <section v-if="handoffs.length" class="stack">
        <h3>{{ t('flow.chain') }}</h3>
        <ol class="history">
          <li v-for="h in handoffs" :key="h.id">{{ h.from_name }} → {{ h.to_name }} <span class="pill" :class="h.state === 'accepted' ? 'level-info' : h.state.startsWith('pending') ? 'level-warn' : 'level-critical'">{{ h.is_return ? t('flow.returned') : t(`flow.state_${h.state}`) }}</span> <span class="muted small">{{ h.reason }}<template v-if="h.decision_note"> · {{ h.decision_note }}</template> · {{ fmt.format(new Date(h.created_at)) }}</span></li>
        </ol>
      </section>

      <section v-if="item.kind !== 'phase'" class="stack box">
        <h3>{{ t('work.logWork') }}</h3>
        <div role="group" :aria-label="t('work.minutes')" class="chips">
          <button v-for="m in [30, 60, 120, 180, 240]" :key="m" type="button" :class="{ primary: !log.custom && log.minutes === m }" :aria-pressed="!log.custom && log.minutes === m" @click="log.minutes = m; log.custom = ''">{{ hours(m) }}{{ t('work.hours') }}</button>
          <input v-model="log.custom" type="number" min="0.1" max="24" step="0.1" :aria-label="`${t('work.other')} (${t('work.hours')})`" :placeholder="t('work.other')" class="custom" />
        </div>
        <label class="field">{{ t('work.whatDone') }}<textarea v-model="log.description" rows="3" maxlength="20000" /></label>
        <label class="field">{{ t('work.attach') }}
          <input ref="fileInput" type="file" multiple accept="image/*,audio/*,video/*,*/*" capture="environment" @change="pickFiles" />
        </label>
        <div class="row">
          <button class="primary" type="button" @click="saveLog">{{ t('work.save') }}</button>
          <button v-if="!timer" type="button" @click="startTimer">{{ t('work.startTimer') }}</button>
          <button v-else-if="timer.work_item_id === item.id" type="button" @click="stopTimer">{{ t('work.stopTimer') }}</button>
        </div>
      </section>

      <section class="stack">
        <h3>{{ t('work.logs') }}</h3>
        <p v-if="!logs.length" class="muted">{{ t('work.noLogs') }}</p>
        <article v-for="l in logs" :key="l.id" class="log">
          <div class="row" style="justify-content: space-between">
            <strong>{{ l.user_name }} · {{ hours(l.minutes) }}{{ t('work.hours') }}</strong>
            <span class="muted small">{{ fmt.format(new Date(l.started_at)) }}<template v-if="l.source === 'timer'"> · ⏱</template></span>
          </div>
          <p v-if="l.description" class="desc">{{ l.description }}</p>
          <div v-if="attFor(l.id).length" class="atts">
            <template v-for="a in attFor(l.id)" :key="a.id">
              <a v-if="a.kind === 'image' && a.mime !== 'image/svg+xml'" :href="url(a)" target="_blank" rel="noopener"><img :src="url(a)" :alt="a.file_name" class="thumb" /></a>
              <audio v-else-if="a.kind === 'audio'" :src="url(a)" controls preload="none" />
              <video v-else-if="a.kind === 'video'" :src="url(a)" controls preload="metadata" class="thumb" />
              <a v-else :href="url(a)" class="badge">{{ a.file_name }}</a>
            </template>
          </div>
          <button v-if="l.user_id === me?.user_id && !l.locked" type="button" class="small-btn" @click="deleteLog(l.id)">{{ t('work.delete') }}</button>
        </article>
        <div v-if="loose.length" class="atts">
          <a v-for="a in loose" :key="a.id" :href="url(a)" class="badge">{{ a.file_name }}</a>
        </div>
      </section>

      <section v-if="item.kind !== 'phase'" class="stack">
        <h3>{{ t('flow.commits') }}</h3>
        <p v-if="!commits.length" class="muted small">{{ t('flow.noCommits') }}</p>
        <ul class="subs">
          <li v-for="c in commits" :key="c.id" class="small"><code>{{ c.sha.slice(0, 8) }}</code> <a v-if="c.url" :href="c.url" target="_blank" rel="noopener noreferrer">{{ c.message.split('\n')[0] || c.sha }}</a><template v-else>{{ c.message.split('\n')[0] }}</template> · {{ c.user_name ?? c.author_name }}<template v-if="c.repo_name"> · {{ c.repo_name }}</template></li>
        </ul>
        <form v-if="isHolder || canManage" class="row" @submit.prevent="linkCommit">
          <input v-model="cm.sha" required pattern="[0-9a-fA-F]{7,64}" :placeholder="t('flow.sha')" :aria-label="t('flow.sha')" style="flex: 0 1 140px" />
          <input v-model="cm.url" type="url" :placeholder="t('flow.commitUrl')" :aria-label="t('flow.commitUrl')" style="flex: 1 1 160px" />
          <button type="submit">{{ t('flow.linkCommit') }}</button>
        </form>
      </section>

      <section class="stack">
        <h3>{{ t('work.history') }}</h3>
        <ol class="history">
          <li v-for="e in events" :key="e.id"><strong>{{ e.actor_name }}</strong> {{ evText(e) }} <span class="muted small">{{ fmt.format(new Date(e.created_at)) }}</span></li>
        </ol>
      </section>
    </template>
  </aside>
</template>

<style scoped>
.panel { display: flex; flex-direction: column; gap: 14px; background: var(--surface); border: 1px solid var(--line); border-radius: 12px; padding: 20px; min-width: 0; }
.grid2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
h3 { margin: 0; font-size: 15px; }
.box { background: var(--inset); border-radius: 10px; padding: 14px; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; }
.chips button { min-width: 52px; }
.custom { width: 90px; }
textarea { padding: 8px 12px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); font: inherit; resize: vertical; }
.log { border-bottom: 1px solid var(--inset); padding-bottom: 10px; display: flex; flex-direction: column; gap: 6px; }
.desc { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
.atts { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.thumb { max-width: 160px; max-height: 110px; border-radius: 6px; border: 1px solid var(--line); object-fit: cover; }
.small { font-size: 12px; }
.spec { border: 1px solid var(--line); border-radius: 10px; padding: 10px 12px; display: flex; flex-direction: column; gap: 6px; }
.spec.met { border-color: var(--accent); background: var(--accent-soft); }
.subs { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; overflow-wrap: anywhere; }
code { font-family: ui-monospace, monospace; background: var(--inset); padding: 0 4px; border-radius: 4px; }
.small-btn { height: 30px; padding: 0 10px; align-self: flex-start; font-size: 12px; }
.history { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; }
input[type="range"] { padding: 0; height: 28px; }
input[type="file"] { padding: 6px 0; height: auto; border: 0; }
@media (max-width: 520px) { .grid2 { grid-template-columns: minmax(0, 1fr); } }
</style>
