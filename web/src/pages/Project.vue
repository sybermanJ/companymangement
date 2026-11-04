<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import Shell from './Shell.vue'
import Gantt from './Gantt.vue'
import ItemPanel from './ItemPanel.vue'
import { api, ApiError, type Dependency, type Item, type Member, type Project } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'

const { t } = useI18n()
const route = useRoute()
const id = computed(() => String(route.params.id))
const project = ref<Project | null>(null)
const items = ref<Item[]>([])
const deps = ref<Dependency[]>([])
const members = ref<Member[]>([])
const canManage = ref(false)
const error = ref('')
const notice = ref('')
const open = ref<string | null>(null)
const scale = ref<'day' | 'week'>('day')

async function load() {
  const d = await api.get<{ project: Project; items: Item[]; dependencies: Dependency[]; can_manage: boolean }>(`/projects/${id.value}`)
  project.value = d.project
  items.value = d.items
  deps.value = d.dependencies
  canManage.value = d.can_manage
}
onMounted(async () => {
  try {
    await load()
    members.value = await api.get<Member[]>('/members')
  } catch (e) {
    error.value = errorText(e, t)
  }
})

const phases = computed(() => items.value.filter((i) => i.kind === 'phase'))
const taskCount = computed(() => items.value.filter((i) => i.kind === 'task'))
const doneCount = computed(() => taskCount.value.filter((i) => i.status === 'done').length)

async function reschedule(it: Item, start: string, end: string) {
  error.value = ''
  try {
    await api.patch(`/items/${it.id}`, { version: it.version, planned_start: start, planned_end: end })
  } catch (e) {
    error.value = e instanceof ApiError && e.status === 409 ? t('work.conflict') : errorText(e, t)
  }
  await load()
}

const f = reactive({ kind: 'task', title: '', parent_id: '', holder_user_id: '', planned_start: '', planned_end: '', estimate: '' })
async function addItem() {
  error.value = ''
  try {
    await api.post(`/projects/${id.value}/items`, {
      kind: f.kind, title: f.title, parent_id: f.parent_id || null, holder_user_id: f.holder_user_id || null,
      planned_start: f.planned_start || null, planned_end: f.planned_end || null,
      estimate_minutes: f.estimate ? Math.round(Number(f.estimate) * 60) : null,
    })
    f.title = ''
    f.estimate = ''
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
const dep = reactive({ from_item: '', to_item: '' })
async function addDep() {
  error.value = ''
  try {
    await api.post('/dependencies', { ...dep })
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
const showForms = ref(false)
</script>

<template>
  <Shell>
    <div class="stack">
      <RouterLink to="/projects" class="muted small">← {{ t('work.back') }}</RouterLink>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <template v-if="project">
        <div class="row" style="justify-content: space-between; align-items: flex-end">
          <div style="min-width: 0">
            <div class="muted small">{{ project.key }} · {{ t('work.owner') }} {{ project.owner_name }}</div>
            <h1>{{ project.name }}</h1>
          </div>
          <div class="row">
            <div role="group" :aria-label="t('work.gantt')" class="seg">
              <button type="button" :class="{ primary: scale === 'day' }" :aria-pressed="scale === 'day'" @click="scale = 'day'">{{ t('work.dayView') }}</button>
              <button type="button" :class="{ primary: scale === 'week' }" :aria-pressed="scale === 'week'" @click="scale = 'week'">{{ t('work.weekView') }}</button>
            </div>
            <button v-if="canManage" class="primary" type="button" @click="showForms = !showForms">+ {{ t('work.addItem') }}</button>
          </div>
        </div>

        <div class="stats">
          <div class="card"><div class="muted small">{{ t('work.logged') }} / {{ t('work.estimate') }}</div><div class="big">{{ hours(project.logged_minutes) }} <span class="muted">/ {{ hours(project.estimate_minutes) }} {{ t('work.hours') }}</span></div></div>
          <div class="card"><div class="muted small">{{ t('work.openItems') }}</div><div class="big">{{ project.open_items }} <span class="muted">/ {{ project.total_items }}</span></div></div>
          <div class="card"><div class="muted small">{{ t('work.s_done') }}</div><div class="big">{{ doneCount }} <span class="muted">/ {{ taskCount.length }}</span></div></div>
          <div v-if="project.labour_budget_minor != null" class="card"><div class="muted small">{{ t('work.budget') }}</div><div class="big">{{ project.currency }} {{ (project.labour_budget_minor / 100).toLocaleString() }}</div></div>
        </div>

        <div v-if="showForms && canManage" class="card stack">
          <form class="row" @submit.prevent="addItem">
            <label class="field">{{ t('work.kind') }}
              <select v-model="f.kind"><option value="task">{{ t('work.task') }}</option><option value="phase">{{ t('work.phase') }}</option><option value="milestone">{{ t('work.milestone') }}</option></select>
            </label>
            <label class="field" style="flex: 1 1 220px">{{ t('work.title') }}<input v-model="f.title" required maxlength="300" /></label>
            <label class="field">{{ t('work.parentItem') }}
              <select v-model="f.parent_id"><option value="">—</option><option v-for="p in phases" :key="p.id" :value="p.id">{{ p.title }}</option></select>
            </label>
            <label class="field">{{ t('work.holder') }}
              <select v-model="f.holder_user_id"><option value="">{{ t('work.unassigned') }}</option><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
            </label>
            <label class="field">{{ t('work.start') }}<input v-model="f.planned_start" type="date" /></label>
            <label class="field">{{ t('work.end') }}<input v-model="f.planned_end" type="date" /></label>
            <label class="field" style="flex: 0 1 120px">{{ t('work.estimateHours') }}<input v-model="f.estimate" type="number" min="0" step="0.5" /></label>
            <button class="primary" type="submit" style="align-self: flex-end">{{ t('common.create') }}</button>
          </form>
          <form class="row" @submit.prevent="addDep">
            <label class="field" style="flex: 1 1 200px">{{ t('work.depFrom') }}
              <select v-model="dep.from_item" required><option v-for="i in items" :key="i.id" :value="i.id">{{ i.key }} {{ i.title }}</option></select>
            </label>
            <label class="field" style="flex: 1 1 200px">{{ t('work.depTo') }}
              <select v-model="dep.to_item" required><option v-for="i in items" :key="i.id" :value="i.id">{{ i.key }} {{ i.title }}</option></select>
            </label>
            <button type="submit" style="align-self: flex-end">{{ t('work.addDep') }}</button>
          </form>
        </div>

        <div class="layout" :class="{ withPanel: !!open }">
          <div class="stack" style="min-width: 0">
            <Gantt :items="items" :dependencies="deps" :can-manage="canManage" :scale="scale" @open="(i) => (open = i.id)" @reschedule="reschedule" />
            <div class="legend muted small">
              <span><i class="sw active" /> {{ t('work.legendActive') }}</span>
              <span><i class="sw done" /> {{ t('work.legendDone') }}</span>
              <span><i class="sw todo" /> {{ t('work.legendTodo') }}</span>
              <span>◆ {{ t('work.milestone') }}</span>
              <span><i class="sw today" /> {{ t('work.today') }}</span>
              <span v-if="canManage">{{ t('work.dragHint') }}</span>
            </div>
          </div>
          <ItemPanel v-if="open" :item-id="open" :members="members" @close="open = null" @changed="load" />
        </div>
      </template>
    </div>
  </Shell>
</template>

<style scoped>
.small { font-size: 12px; }
.seg { display: flex; }
.seg button:first-child { border-radius: 8px 0 0 8px; }
.seg button:last-child { border-radius: 0 8px 8px 0; margin-left: -1px; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; }
.stats .card { padding: 14px 16px; }
.big { font-size: 22px; font-weight: 700; margin-top: 4px; }
.big .muted { font-size: 14px; font-weight: 400; }
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(320px, 420px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
.legend { display: flex; flex-wrap: wrap; gap: 8px 18px; }
.sw { display: inline-block; width: 18px; height: 10px; border-radius: 3px; vertical-align: middle; }
.sw.active { background: var(--accent-soft); border: 1px solid var(--accent); }
.sw.done { background: var(--line); }
.sw.todo { border: 1px dashed var(--accent); }
.sw.today { width: 2px; height: 12px; background: var(--danger); }
</style>
