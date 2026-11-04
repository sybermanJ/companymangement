<script setup lang="ts">
// Side panel for one work item: status, dates, progress, logging with files, timer, history.
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, uploadFiles, ApiError, type Attachment, type Item, type ItemEvent, type Member, type Timer, type WorkLog } from '../lib/api'
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
const timer = ref<Timer | null>(null)
const error = ref('')
const notice = ref('')
const STATUSES = ['todo', 'in_progress', 'blocked', 'in_review', 'done', 'cancelled']

const isHolder = computed(() => item.value?.holder_user_id === me.value?.user_id)
const canEditStatus = computed(() => canManage.value || isHolder.value)
const fmt = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))

async function load() {
  const d = await api.get<{ item: Item; logs: WorkLog[]; attachments: Attachment[]; events: ItemEvent[]; can_manage: boolean }>(`/items/${props.itemId}`)
  item.value = d.item
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

const setStatus = (status: string) => act(() => api.post(`/items/${props.itemId}/status`, { version: item.value!.version, status }))
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
.small-btn { height: 30px; padding: 0 10px; align-self: flex-start; font-size: 12px; }
.history { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; }
input[type="range"] { padding: 0; height: 28px; }
input[type="file"] { padding: 6px 0; height: auto; border: 0; }
@media (max-width: 520px) { .grid2 { grid-template-columns: minmax(0, 1fr); } }
</style>
