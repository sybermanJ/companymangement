<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import Shell from './Shell.vue'
import ItemPanel from './ItemPanel.vue'
import { api, type Handoff, type Item, type Member, type Project, type Timer } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'
import { me } from '../lib/session'

const { t } = useI18n()
const route = useRoute()
const items = ref<Item[]>([])
const claimable = ref<Item[]>([])
const sent = ref<Handoff[]>([])
const timer = ref<Timer | null>(null)
const members = ref<Member[]>([])
const projects = ref<Project[]>([])
const open = ref<string | null>(typeof route.query.item === 'string' ? route.query.item : null)
const error = ref('')
const notice = ref('')
const showReq = ref(false)
const req = reactive({ to_user: '', title: '', description: '', due: '', project_id: '' })

watch(() => route.query.item, (v) => { if (typeof v === 'string') open.value = v })

async function load() {
  const d = await api.get<{ items: Item[]; timer: Timer | null; claimable: Item[]; sent_handoffs: Handoff[] }>('/my/items')
  items.value = d.items
  timer.value = d.timer
  claimable.value = d.claimable
  sent.value = d.sent_handoffs
}
onMounted(async () => {
  try {
    await load()
    ;[members.value, projects.value] = await Promise.all([api.get<Member[]>('/members'), api.get<Project[]>('/projects')])
  } catch (e) {
    error.value = errorText(e, t)
  }
})
const running = computed(() => items.value.find((i) => i.id === timer.value?.work_item_id))
const groups = computed(() => [
  { k: 'routine', list: items.value.filter((i) => i.kind === 'routine') },
  { k: 'request', list: items.value.filter((i) => i.kind === 'request') },
  { k: 'project', list: items.value.filter((i) => i.kind !== 'routine' && i.kind !== 'request') },
].filter((g) => g.list.length))

async function run(fn: () => Promise<unknown>, ok?: string) {
  error.value = ''
  notice.value = ''
  try {
    await fn()
    if (ok) notice.value = ok
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
const claim = (i: Item) => run(async () => { await api.post(`/items/${i.id}/claim`); open.value = i.id }, t('flow.claimed'))
const cancel = (h: Handoff) => run(() => api.post(`/handoffs/${h.id}/decide`, { action: 'cancel' }))
const sendRequest = () => run(async () => {
  await api.post('/requests', { to_user: req.to_user, title: req.title, description: req.description, due: req.due || null, project_id: req.project_id || null })
  Object.assign(req, { to_user: '', title: '', description: '', due: '', project_id: '' })
  showReq.value = false
}, t('flow.requestSent'))
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('work.mine') }}</h1>
        <button class="primary" type="button" @click="showReq = !showReq">+ {{ t('flow.newRequest') }}</button>
      </div>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
      <p v-if="running" class="notice ok">{{ t('work.timerRunning', { key: running.key, title: running.title }) }}</p>

      <form v-if="showReq" class="card stack" @submit.prevent="sendRequest">
        <h2>{{ t('flow.newRequest') }}</h2>
        <p class="muted small">{{ t('flow.requestHint') }}</p>
        <div class="row">
          <label class="field" style="flex: 1 1 200px">{{ t('flow.to') }}
            <select v-model="req.to_user" required><option value="">{{ t('org.choose') }}</option><option v-for="m in members.filter((x) => x.status === 'active' && x.user_id !== me?.user_id)" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
          </label>
          <label class="field" style="flex: 2 1 260px">{{ t('work.title') }}<input v-model="req.title" required maxlength="300" /></label>
          <label class="field">{{ t('flow.due') }}<input v-model="req.due" type="date" /></label>
          <label class="field">{{ t('flow.project') }}
            <select v-model="req.project_id"><option value="">{{ t('common.none') }}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{ p.key }} {{ p.name }}</option></select>
          </label>
        </div>
        <label class="field">{{ t('work.description') }}<textarea v-model="req.description" rows="3" maxlength="20000" /></label>
        <div class="row"><button class="primary" type="submit">{{ t('flow.send') }}</button><button type="button" @click="showReq = false">{{ t('common.cancel') }}</button></div>
      </form>

      <div class="layout" :class="{ withPanel: !!open }">
        <div class="stack" style="min-width: 0">
          <section v-if="claimable.length" class="stack">
            <h2>{{ t('flow.claimable') }}</h2>
            <ul class="list">
              <li v-for="i in claimable" :key="i.id" class="card itemrow static">
                <span class="muted small">{{ i.key }}</span>
                <span class="ttl">{{ i.title }}</span>
                <button type="button" class="primary" @click="claim(i)">{{ t('flow.claim') }}</button>
              </li>
            </ul>
          </section>
          <p v-if="!items.length && !claimable.length && !error" class="muted">{{ t('work.noMine') }}</p>
          <section v-for="g in groups" :key="g.k" class="stack">
            <h2>{{ t(`flow.group_${g.k}`) }}</h2>
            <ul class="list">
              <li v-for="i in g.list" :key="i.id">
                <button type="button" class="card itemrow" :class="{ on: open === i.id }" @click="open = i.id">
                  <span class="muted small">{{ i.key }}</span>
                  <span class="ttl">{{ i.title }}</span>
                  <span v-if="i.open_handoff" class="pill level-warn">{{ t(`flow.state_${i.open_handoff}`) }}</span>
                  <span class="badge">{{ t(`work.s_${i.status}`) }}</span>
                  <span class="muted small">{{ i.planned_end ?? '' }}</span>
                  <span class="muted small">{{ hours(i.logged_minutes) }}{{ t('work.hours') }}</span>
                </button>
              </li>
            </ul>
          </section>
          <section v-if="sent.length" class="stack">
            <h2>{{ t('flow.sent') }}</h2>
            <ul class="list">
              <li v-for="h in sent" :key="h.id" class="card itemrow static">
                <span class="muted small">{{ h.item_key }}</span>
                <span class="ttl">{{ h.item_title }} → {{ h.to_name }}</span>
                <span class="pill level-warn">{{ t(`flow.state_${h.state}`) }}</span>
                <button type="button" @click="cancel(h)">{{ t('flow.cancelHandoff') }}</button>
              </li>
            </ul>
          </section>
        </div>
        <ItemPanel v-if="open" :item-id="open" :members="members" @close="open = null" @changed="load" />
      </div>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
.itemrow { width: 100%; height: auto; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; text-align: left; padding: 12px 16px; }
.itemrow.on { border-color: var(--accent); }
.ttl { flex: 1 1 200px; min-width: 0; overflow-wrap: anywhere; }
textarea { padding: 8px 12px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); font: inherit; resize: vertical; }
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(320px, 440px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
</style>
