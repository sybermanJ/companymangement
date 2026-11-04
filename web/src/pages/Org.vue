<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import { api, type Assignment, type Department, type Member, type Role } from '../lib/api'
import { currentMembership } from '../lib/session'
import { errorText } from '../lib/errors'

const { t } = useI18n()
type Tab = 'departments' | 'members' | 'roles' | 'git'
const tabs: Tab[] = ['departments', 'members', 'roles', 'git']
const tab = ref<Tab>('departments')

const departments = ref<Department[]>([])
const members = ref<Member[]>([])
const roles = ref<Role[]>([])
const assignments = ref<Assignment[]>([])
const error = ref('')
const notice = ref('')
const isAdmin = computed(() => currentMembership()?.is_admin ?? false)

const activeMembers = computed(() => members.value.filter((m) => m.status === 'active'))
const nameOf = (id: string | null) => members.value.find((m) => m.user_id === id)?.display_name ?? t('org.noHead')
const deptName = (id: string | null) => departments.value.find((d) => d.id === id)?.name ?? t('org.companyWide')

/** Departments in tree order with depth, for indentation. */
const tree = computed(() => {
  const out: { d: Department; depth: number }[] = []
  const walk = (parent: string | null, depth: number) => {
    for (const d of departments.value.filter((x) => x.parent_id === parent)) {
      out.push({ d, depth })
      walk(d.id, depth + 1)
    }
  }
  walk(null, 0)
  return out
})

async function load() {
  try {
    const [d, m, r] = await Promise.all([
      api.get<Department[]>('/departments'),
      api.get<Member[]>('/members'),
      api.get<{ roles: Role[]; assignments: Assignment[] }>('/roles'),
    ])
    departments.value = d
    members.value = m
    roles.value = r.roles
    assignments.value = r.assignments
  } catch (e) {
    error.value = errorText(e, t)
  }
}
onMounted(load)

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

const newDept = reactive({ name: '', parent_id: '' })
const addDept = () => run(async () => {
  await api.post('/departments', { name: newDept.name, parent_id: newDept.parent_id || null })
  newDept.name = ''
})
const setHead = (id: string, head: string) => run(() => api.put(`/departments/${id}/head`, { head_user_id: head || null }))

const inv = reactive({ email: '', name: '', is_admin: false, is_finance: false })
const invite = () => {
  const email = inv.email
  return run(async () => {
    await api.post('/invitations', { ...inv })
    Object.assign(inv, { email: '', name: '', is_admin: false, is_finance: false })
  }, t('org.inviteSent', { email }))
}

const newRole = reactive({ name: '', department_id: '' })
const addRole = () => run(async () => {
  await api.post('/roles', { name: newRole.name, department_id: newRole.department_id || null })
  newRole.name = ''
})
const asg = reactive({ user_id: '', role_id: '', department_id: '' })
const assign = () => run(() => api.post('/role-assignments', { ...asg }))

const setFlags = (m: Member, is_admin: boolean, is_finance: boolean) => run(() => api.put(`/members/${m.user_id}/flags`, { is_admin, is_finance }), t('work.saved'))

interface Repo { repo: { id: string; provider: string; name: string; url: string; project_id: string | null; commits: number }; webhook_url: string }
const git = ref<{ repos: Repo[]; identities: { email: string; user_name: string }[]; unmapped: { email: string; name: string; commits: number }[] }>({ repos: [], identities: [], unmapped: [] })
const created = ref<{ secret: string; webhook_url: string } | null>(null)
const nr = reactive({ provider: 'github', name: '', url: '' })
const loadGit = async () => {
  if (isAdmin.value) git.value = await api.get('/git/repos')
}
watch(tab, (v) => { if (v === 'git') loadGit().catch((e) => (error.value = errorText(e, t))) })
const addRepo = () => run(async () => {
  created.value = await api.post('/git/repos', { ...nr })
  Object.assign(nr, { provider: 'github', name: '', url: '' })
  await loadGit()
})
const delRepo = (r: Repo) => run(async () => { await api.del(`/git/repos/${r.repo.id}`); await loadGit() })
const mapTo = reactive<Record<string, string>>({})
const mapEmail = (email: string) => run(async () => { await api.post('/git/identities', { email, user_id: mapTo[email] }); await loadGit() }, t('work.saved'))

const statusLabel = (s: string) => t(s === 'active' ? 'org.statusActive' : s === 'invited' ? 'org.statusInvited' : 'org.statusDeactivated')
</script>

<template>
  <Shell>
    <div class="stack">
      <h1>{{ t('org.title') }}</h1>
      <div role="tablist" class="tabs" :aria-label="t('org.title')">
        <button v-for="k in tabs" :key="k" role="tab" type="button" :aria-selected="tab === k" :class="{ on: tab === k }" @click="tab = k">{{ t(`org.${k}`) }}</button>
      </div>
      <p v-if="!isAdmin" class="notice warn">{{ t('org.adminOnly') }}</p>
      <p v-if="notice" class="notice ok" role="status">{{ notice }}</p>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>

      <section v-if="tab === 'departments'" class="card stack" role="tabpanel">
        <form v-if="isAdmin" class="row" @submit.prevent="addDept">
          <label class="field grow">{{ t('org.deptName') }}<input v-model="newDept.name" required maxlength="100" /></label>
          <label class="field">{{ t('org.parent') }}
            <select v-model="newDept.parent_id">
              <option value="">{{ t('common.none') }}</option>
              <option v-for="d in departments" :key="d.id" :value="d.id">{{ d.name }}</option>
            </select>
          </label>
          <button class="primary end" type="submit">{{ t('org.addDept') }}</button>
        </form>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('org.departments') }}</th><th>{{ t('org.head') }}</th></tr></thead>
            <tbody>
              <tr v-for="{ d, depth } in tree" :key="d.id">
                <td :style="{ paddingLeft: `${8 + depth * 20}px` }">{{ d.name }}</td>
                <td>
                  <select v-if="isAdmin" :aria-label="`${t('org.head')} · ${d.name}`" :value="d.head_user_id ?? ''" @change="setHead(d.id, ($event.target as HTMLSelectElement).value)">
                    <option value="">{{ t('org.noHead') }}</option>
                    <option v-for="m in activeMembers" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option>
                  </select>
                  <span v-else>{{ nameOf(d.head_user_id) }}</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section v-if="tab === 'members'" class="card stack" role="tabpanel">
        <form v-if="isAdmin" class="row" @submit.prevent="invite">
          <label class="field grow">{{ t('auth.email') }}<input v-model="inv.email" type="email" required /></label>
          <label class="field grow">{{ t('org.inviteeName') }}<input v-model="inv.name" required maxlength="100" /></label>
          <label class="check"><input v-model="inv.is_admin" type="checkbox" /> {{ t('org.admin') }}</label>
          <label class="check"><input v-model="inv.is_finance" type="checkbox" /> {{ t('org.finance') }}</label>
          <button class="primary end" type="submit">{{ t('org.invite') }}</button>
        </form>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('org.person') }}</th><th>{{ t('auth.email') }}</th><th>{{ t('org.status') }}</th><th>{{ t('org.roles') }}</th></tr></thead>
            <tbody>
              <tr v-for="m in members" :key="m.user_id">
                <td>{{ m.display_name }}
                  <template v-if="!isAdmin || m.status !== 'active'">
                    <span v-if="m.is_admin" class="badge accent">{{ t('org.admin') }}</span>
                    <span v-if="m.is_finance" class="badge">{{ t('org.finance') }}</span>
                  </template>
                  <span v-else class="flags">
                    <label class="check small"><input type="checkbox" :checked="m.is_admin" @change="setFlags(m, ($event.target as HTMLInputElement).checked, m.is_finance)" /> {{ t('org.admin') }}</label>
                    <label class="check small"><input type="checkbox" :checked="m.is_finance" @change="setFlags(m, m.is_admin, ($event.target as HTMLInputElement).checked)" /> {{ t('org.finance') }}</label>
                  </span>
                </td>
                <td>{{ m.email }}</td>
                <td><span class="badge" :class="{ accent: m.status === 'active' }">{{ statusLabel(m.status) }}</span></td>
                <td>{{ assignments.filter((a) => a.user_id === m.user_id).map((a) => `${a.role_name} · ${a.department_name}`).join('，') }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section v-if="tab === 'roles'" class="card stack" role="tabpanel">
        <form v-if="isAdmin" class="row" @submit.prevent="addRole">
          <label class="field grow">{{ t('org.roleName') }}<input v-model="newRole.name" required maxlength="100" /></label>
          <label class="field">{{ t('org.roleDept') }}
            <select v-model="newRole.department_id">
              <option value="">{{ t('org.companyWide') }}</option>
              <option v-for="d in departments" :key="d.id" :value="d.id">{{ d.name }}</option>
            </select>
          </label>
          <button class="primary end" type="submit">{{ t('org.addRole') }}</button>
        </form>
        <form v-if="isAdmin" class="row" @submit.prevent="assign">
          <label class="field">{{ t('org.person') }}
            <select v-model="asg.user_id" required>
              <option value="" disabled>{{ t('org.choose') }}</option>
              <option v-for="m in activeMembers" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option>
            </select>
          </label>
          <label class="field">{{ t('org.roles') }}
            <select v-model="asg.role_id" required>
              <option value="" disabled>{{ t('org.choose') }}</option>
              <option v-for="r in roles" :key="r.id" :value="r.id">{{ r.name }}</option>
            </select>
          </label>
          <label class="field">{{ t('org.departments') }}
            <select v-model="asg.department_id" required>
              <option value="" disabled>{{ t('org.choose') }}</option>
              <option v-for="d in departments" :key="d.id" :value="d.id">{{ d.name }}</option>
            </select>
          </label>
          <button class="primary end" type="submit">{{ t('org.assign') }}</button>
        </form>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('org.roleName') }}</th><th>{{ t('org.departments') }}</th><th>{{ t('org.members') }}</th></tr></thead>
            <tbody>
              <tr v-for="r in roles" :key="r.id">
                <td>{{ r.name }}</td>
                <td>{{ deptName(r.department_id) }}</td>
                <td>{{ assignments.filter((a) => a.role_id === r.id).map((a) => nameOf(a.user_id)).join('，') }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
      <section v-if="tab === 'git'" class="card stack" role="tabpanel">
        <p class="muted">{{ t('git.intro') }}</p>
        <form v-if="isAdmin" class="row" @submit.prevent="addRepo">
          <label class="field">{{ t('git.provider') }}
            <select v-model="nr.provider"><option value="github">GitHub</option><option value="gitea">Gitea / Forgejo</option><option value="gitlab">GitLab</option><option value="generic">{{ t('git.other') }}</option></select>
          </label>
          <label class="field grow">{{ t('git.name') }}<input v-model="nr.name" required maxlength="200" placeholder="acme/pay" /></label>
          <label class="field grow">{{ t('git.url') }}<input v-model="nr.url" type="url" placeholder="https://" /></label>
          <button class="primary end" type="submit">{{ t('git.add') }}</button>
        </form>
        <div v-if="created" class="notice warn stack">
          <strong>{{ t('git.copyNow') }}</strong>
          <div>{{ t('git.webhookUrl') }}: <code>{{ created.webhook_url }}</code></div>
          <div>{{ t('git.secret') }}: <code>{{ created.secret }}</code></div>
          <div class="small">{{ t('git.howTo') }}</div>
        </div>
        <div class="table-wrap">
          <table>
            <thead><tr><th>{{ t('git.name') }}</th><th>{{ t('git.provider') }}</th><th>{{ t('git.webhookUrl') }}</th><th>{{ t('git.commits') }}</th><th></th></tr></thead>
            <tbody>
              <tr v-for="r in git.repos" :key="r.repo.id">
                <td><a v-if="r.repo.url" :href="r.repo.url" target="_blank" rel="noopener noreferrer">{{ r.repo.name }}</a><template v-else>{{ r.repo.name }}</template></td>
                <td>{{ r.repo.provider }}</td>
                <td class="small"><code>{{ r.webhook_url }}</code></td>
                <td>{{ r.repo.commits }}</td>
                <td><button type="button" @click="delRepo(r)">{{ t('work.delete') }}</button></td>
              </tr>
            </tbody>
          </table>
        </div>
        <h2 class="sub">{{ t('git.authors') }}</h2>
        <p class="muted small">{{ t('git.authorsHelp') }}</p>
        <ul class="plain">
          <li v-for="i in git.identities" :key="i.email"><code>{{ i.email }}</code> → {{ i.user_name }}</li>
        </ul>
        <div v-for="u in git.unmapped" :key="u.email" class="row">
          <span style="flex: 1 1 240px"><code>{{ u.email }}</code> {{ u.name }} · {{ t('git.nCommits', { n: u.commits }) }}</span>
          <select v-model="mapTo[u.email]" :aria-label="t('org.person')"><option value="">{{ t('org.choose') }}</option><option v-for="m in activeMembers" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
          <button type="button" :disabled="!mapTo[u.email]" @click="mapEmail(u.email)">{{ t('git.map') }}</button>
        </div>
      </section>
    </div>
  </Shell>
</template>

<style scoped>
.flags { display: inline-flex; gap: 10px; margin-left: 8px; }
.flags .check { height: auto; align-self: center; }
code { font-size: 12px; overflow-wrap: anywhere; }
.sub { font-size: 15px; margin: 8px 0 0; }
.plain { margin: 0; padding-left: 18px; }
.grow { flex: 1 1 200px; }
.end { align-self: flex-end; }
.check { display: flex; align-items: center; gap: 6px; align-self: flex-end; height: 40px; }
.check input { height: auto; }
</style>
