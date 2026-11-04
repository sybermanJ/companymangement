<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import { api, type Member, type Project } from '../lib/api'
import { currentMembership } from '../lib/session'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'

const { t } = useI18n()
const projects = ref<Project[]>([])
const members = ref<Member[]>([])
const error = ref('')
const isAdmin = computed(() => currentMembership()?.is_admin ?? false)
const showForm = ref(false)
const f = reactive({ key: '', name: '', owner_id: '', planned_start: '', planned_end: '', currency: '', budget: '' })

async function load() {
  try {
    ;[projects.value, members.value] = await Promise.all([api.get<Project[]>('/projects'), api.get<Member[]>('/members')])
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(load)

async function create() {
  error.value = ''
  try {
    await api.post('/projects', {
      key: f.key, name: f.name, owner_id: f.owner_id || undefined,
      planned_start: f.planned_start || null, planned_end: f.planned_end || null,
      currency: f.currency || undefined,
      labour_budget_minor: f.budget ? Math.round(Number(f.budget) * 100) : null,
    })
    Object.assign(f, { key: '', name: '', owner_id: '', planned_start: '', planned_end: '', currency: '', budget: '' })
    showForm.value = false
    await load()
  } catch (e) {
    error.value = errorText(e, t)
  }
}
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('work.projects') }}</h1>
        <button v-if="isAdmin" class="primary" type="button" @click="showForm = !showForm">+ {{ t('work.newProject') }}</button>
      </div>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <form v-if="showForm" class="card row" @submit.prevent="create">
        <label class="field" style="flex: 0 1 140px">{{ t('work.key') }}<input v-model="f.key" required pattern="[A-Za-z][A-Za-z0-9]{1,9}" placeholder="PAY" /></label>
        <label class="field" style="flex: 1 1 220px">{{ t('work.name') }}<input v-model="f.name" required maxlength="200" /></label>
        <label class="field">{{ t('work.owner') }}
          <select v-model="f.owner_id"><option value="">—</option><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
        </label>
        <label class="field">{{ t('work.start') }}<input v-model="f.planned_start" type="date" /></label>
        <label class="field">{{ t('work.end') }}<input v-model="f.planned_end" type="date" /></label>
        <label class="field" style="flex: 0 1 100px">{{ t('auth.currency') }}<input v-model="f.currency" maxlength="3" placeholder="CNY" /></label>
        <label class="field" style="flex: 0 1 160px">{{ t('work.budget') }}<input v-model="f.budget" type="number" min="0" step="0.01" /></label>
        <button class="primary" type="submit" style="align-self: flex-end">{{ t('common.create') }}</button>
      </form>
      <p v-if="!projects.length && !error" class="muted">{{ t('work.noProjects') }}</p>
      <div class="cards">
        <RouterLink v-for="p in projects" :key="p.id" :to="`/projects/${p.id}`" class="card pcard">
          <div class="muted small">{{ p.key }} · {{ p.owner_name }}</div>
          <h2>{{ p.name }}</h2>
          <div class="muted small">{{ p.planned_start ?? '—' }} → {{ p.planned_end ?? '—' }}</div>
          <div class="row stats">
            <div><div class="muted small">{{ t('work.logged') }} / {{ t('work.estimate') }}</div><strong>{{ hours(p.logged_minutes) }} / {{ hours(p.estimate_minutes) }} {{ t('work.hours') }}</strong></div>
            <div><div class="muted small">{{ t('work.openItems') }}</div><strong>{{ p.open_items }} / {{ p.total_items }}</strong></div>
          </div>
        </RouterLink>
      </div>
    </div>
  </Shell>
</template>

<style scoped>
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px; }
.pcard { display: flex; flex-direction: column; gap: 6px; text-decoration: none; color: var(--text); }
.pcard:hover { border-color: var(--accent); }
.small { font-size: 12px; }
.stats { gap: 24px; margin-top: 6px; }
</style>
