<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import { api, type Assignment, type Department, type Item, type Member, type Project, type Role, type Spec } from '../lib/api'
import { errorText } from '../lib/errors'
import { currentMembership } from '../lib/session'

interface Template {
  id: string; title: string; description: string; recurrence: string; weekdays: number[]
  target_user_id: string | null; target_user_name: string | null; target_role_id: string | null; target_role_name: string | null
  target_department_id: string | null; target_department_name: string | null; mode: string; estimate_minutes: number | null
  active: boolean; created_by_name: string; open_items: number
}
interface Row { template: Template; deliverables: Spec[]; can_edit: boolean }
interface Missing { item: Item; missing: { kind: string; needed: number; have: number }[] }

const { t } = useI18n()
const rows = ref<Row[]>([])
const missing = ref<Missing[]>([])
const canCreate = ref(false)
const headed = ref<string[]>([])
const members = ref<Member[]>([])
const roles = ref<Role[]>([])
const departments = ref<Department[]>([])
const projects = ref<Project[]>([])
const error = ref('')
const notice = ref('')
const showForm = ref(false)
const KINDS = ['text', 'image', 'audio', 'video', 'file', 'git_commit', 'link', 'checklist']

const f = reactive({
  title: '', description: '', target: 'role' as 'role' | 'user', user: '', role: '', dept: '', mode: 'each_member',
  recurrence: 'weekdays', weekdays: [1] as number[], estimate: '', project: '',
  specs: [{ kind: 'text', min_count: 1, required: true, instructions: '' }] as { kind: string; min_count: number; required: boolean; instructions: string }[],
})
const deptName = (id: string | null) => departments.value.find((d) => d.id === id)?.name ?? ''
const isAdmin = computed(() => currentMembership()?.is_admin ?? false)
const myDepts = computed(() => departments.value.filter((d) => headed.value.includes(d.id) || isAdmin.value))
const rolesIn = computed(() => roles.value.filter((r) => !r.department_id || r.department_id === f.dept))

async function load() {
  const d = await api.get<{ templates: Row[]; can_create: boolean; headed_departments: string[]; missing: Missing[] }>('/routines')
  rows.value = d.templates
  canCreate.value = d.can_create
  headed.value = d.headed_departments
  missing.value = d.missing
}
onMounted(async () => {
  try {
    await load()
    const [m, r, dd, p] = await Promise.all([
      api.get<Member[]>('/members'), api.get<{ roles: Role[]; assignments: Assignment[] }>('/roles'),
      api.get<Department[]>('/departments'), api.get<Project[]>('/projects'),
    ])
    members.value = m
    roles.value = r.roles
    departments.value = dd
    projects.value = p
  } catch (e) {
    error.value = errorText(e, t)
  }
})

function recText(x: Template) {
  if (x.recurrence === 'weekly') return t('flow.rec_weekly', { days: x.weekdays.map((d) => t(`flow.wd${d}`)).join('、') })
  return t(`flow.rec_${x.recurrence}`)
}
function target(x: Template) {
  return x.target_user_name ?? `${x.target_role_name} @ ${x.target_department_name}`
}

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
const create = () => run(async () => {
  await api.post('/routines', {
    title: f.title, description: f.description, recurrence: f.recurrence, weekdays: f.recurrence === 'weekly' ? f.weekdays : [],
    target_user_id: f.target === 'user' ? f.user : null,
    target_role_id: f.target === 'role' ? f.role : null,
    target_department_id: f.target === 'role' ? f.dept : null,
    mode: f.target === 'role' ? f.mode : 'each_member',
    estimate_minutes: f.estimate ? Math.round(Number(f.estimate) * 60) : null,
    project_id: f.project || null,
    deliverables: f.specs.filter((s) => s.kind),
  })
  showForm.value = false
  Object.assign(f, { title: '', description: '' })
}, t('flow.routineCreated'))
const toggle = (r: Row) => run(() => api.post(`/routines/${r.template.id}/active`, { active: !r.template.active }))
const toggleDay = (d: number) => {
  f.weekdays = f.weekdays.includes(d) ? f.weekdays.filter((x) => x !== d) : [...f.weekdays, d].sort()
}
</script>

<template>
  <Shell>
    <div class="stack">
      <div class="row" style="justify-content: space-between">
        <h1>{{ t('nav.routines') }}</h1>
        <button v-if="canCreate" class="primary" type="button" @click="showForm = !showForm">+ {{ t('flow.newRoutine') }}</button>
      </div>
      <p class="muted">{{ t('flow.routinesIntro') }}</p>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>

      <form v-if="showForm" class="card stack" @submit.prevent="create">
        <div class="row">
          <label class="field" style="flex: 2 1 260px">{{ t('work.title') }}<input v-model="f.title" required maxlength="200" :placeholder="t('flow.routineExample')" /></label>
          <label class="field">{{ t('flow.repeat') }}
            <select v-model="f.recurrence"><option value="daily">{{ t('flow.rec_daily') }}</option><option value="weekdays">{{ t('flow.rec_weekdays') }}</option><option value="weekly">{{ t('flow.weekly') }}</option></select>
          </label>
          <label class="field" style="flex: 0 1 140px">{{ t('work.estimateHours') }}<input v-model="f.estimate" type="number" min="0" max="24" step="0.25" /></label>
        </div>
        <fieldset v-if="f.recurrence === 'weekly'" class="row days">
          <legend class="muted small">{{ t('flow.onDays') }}</legend>
          <label v-for="d in [1, 2, 3, 4, 5, 6, 7]" :key="d" class="chk"><input type="checkbox" :checked="f.weekdays.includes(d)" @change="toggleDay(d)" />{{ t(`flow.wd${d}`) }}</label>
        </fieldset>
        <fieldset class="row">
          <legend class="muted small">{{ t('flow.assignTo') }}</legend>
          <label class="chk"><input v-model="f.target" type="radio" value="role" />{{ t('flow.aRole') }}</label>
          <label class="chk"><input v-model="f.target" type="radio" value="user" />{{ t('flow.aPerson') }}</label>
        </fieldset>
        <div v-if="f.target === 'role'" class="row">
          <label class="field">{{ t('org.departments') }}
            <select v-model="f.dept" required><option value="">{{ t('org.choose') }}</option><option v-for="d in myDepts" :key="d.id" :value="d.id">{{ d.name }}</option></select>
          </label>
          <label class="field">{{ t('org.roles') }}
            <select v-model="f.role" required><option value="">{{ t('org.choose') }}</option><option v-for="r in rolesIn" :key="r.id" :value="r.id">{{ r.name }}<template v-if="r.department_id"> · {{ deptName(r.department_id) }}</template></option></select>
          </label>
          <label class="field">{{ t('flow.mode') }}
            <select v-model="f.mode"><option value="each_member">{{ t('flow.mode_each_member') }}</option><option value="pool">{{ t('flow.mode_pool') }}</option></select>
          </label>
        </div>
        <div v-else class="row">
          <label class="field">{{ t('org.person') }}
            <select v-model="f.user" required><option value="">{{ t('org.choose') }}</option><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
          </label>
        </div>
        <div class="row">
          <label class="field" style="flex: 1 1 260px">{{ t('work.description') }}<input v-model="f.description" maxlength="2000" /></label>
          <label class="field">{{ t('flow.chargeTo') }}
            <select v-model="f.project"><option value="">{{ t('common.none') }}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{ p.key }} {{ p.name }}</option></select>
          </label>
        </div>
        <fieldset class="stack">
          <legend class="muted small">{{ t('flow.requiredOutputs') }}</legend>
          <div v-for="(s, i) in f.specs" :key="i" class="row">
            <select v-model="s.kind" :aria-label="t('flow.kind')"><option v-for="k in KINDS" :key="k" :value="k">{{ t(`flow.k_${k}`) }}</option></select>
            <input v-model.number="s.min_count" type="number" min="1" max="100" :aria-label="t('flow.minCount')" style="width: 80px" />
            <label class="chk"><input v-model="s.required" type="checkbox" />{{ t('flow.required') }}</label>
            <input v-model="s.instructions" :placeholder="t('flow.instructions')" :aria-label="t('flow.instructions')" style="flex: 1 1 200px" />
            <button type="button" :aria-label="t('work.delete')" @click="f.specs.splice(i, 1)">✕</button>
          </div>
          <div><button type="button" @click="f.specs.push({ kind: 'image', min_count: 1, required: true, instructions: '' })">+ {{ t('flow.addDeliverable') }}</button></div>
        </fieldset>
        <div class="row"><button class="primary" type="submit">{{ t('common.create') }}</button><button type="button" @click="showForm = false">{{ t('common.cancel') }}</button></div>
      </form>

      <section v-if="missing.length" class="card stack">
        <h2>{{ t('flow.missingTitle') }}</h2>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('flow.date') }}</th><th>{{ t('work.title') }}</th><th>{{ t('work.holder') }}</th><th>{{ t('flow.missing') }}</th></tr></thead>
            <tbody>
              <tr v-for="m in missing" :key="m.item.id">
                <td>{{ m.item.occurrence_date }}</td>
                <td><RouterLink :to="`/my?item=${m.item.id}`">{{ m.item.key }} {{ m.item.title }}</RouterLink></td>
                <td>{{ m.item.holder_name ?? t('flow.pool') }}</td>
                <td>{{ m.missing.map((x) => `${t(`flow.k_${x.kind}`)} ${x.have}/${x.needed}`).join('，') }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <p v-if="!rows.length" class="empty">{{ t('flow.noRoutines') }}</p>
      <div class="grid">
        <article v-for="r in rows" :key="r.template.id" class="card stack" :class="{ off: !r.template.active }">
          <div class="row" style="justify-content: space-between">
            <h3>{{ r.template.title }}</h3>
            <span class="pill" :class="r.template.active ? 'level-info' : 'level-warn'">{{ r.template.active ? t('flow.active') : t('flow.paused') }}</span>
          </div>
          <div class="muted small">{{ recText(r.template) }} · {{ target(r.template) }} · {{ t(`flow.mode_${r.template.mode}`) }}</div>
          <ul class="specs">
            <li v-for="s in r.deliverables" :key="s.id">{{ t(`flow.k_${s.kind}`) }}<template v-if="s.min_count > 1"> ×{{ s.min_count }}</template><template v-if="!s.required"> ({{ t('flow.optional') }})</template><template v-if="s.instructions"> — {{ s.instructions }}</template></li>
          </ul>
          <div class="row" style="justify-content: space-between">
            <span class="muted small">{{ t('flow.openItems', { n: r.template.open_items }) }} · {{ r.template.created_by_name }}</span>
            <button v-if="r.can_edit" type="button" @click="toggle(r)">{{ r.template.active ? t('flow.pause') : t('flow.resume') }}</button>
          </div>
        </article>
      </div>
    </div>
  </Shell>
</template>

<style scoped>
h2 { font-size: 16px; margin: 0; }
h3 { font-size: 15px; margin: 0; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px; }
.off { opacity: 0.7; }
.specs { margin: 0; padding-left: 18px; font-size: 13px; }
fieldset { border: 0; padding: 0; margin: 0; }
legend { padding: 0 0 4px; }
.chk { display: inline-flex; align-items: center; gap: 6px; }
.chk input { height: auto; }
</style>
