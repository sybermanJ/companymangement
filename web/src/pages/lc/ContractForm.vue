<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, type Member, type Project } from '../../lib/api'
import { errorText } from '../../lib/errors'
import { toMajor, toMinor } from '../../lib/money'
import { me, currentMembership } from '../../lib/session'
import { CONTRACT_KINDS, uploadDocs, type Contract } from '../../lib/lifecycle'

const props = defineProps<{ members: Member[]; contract?: Contract | null; project?: string }>()
const emit = defineEmits<{ saved: [id: string]; cancel: [] }>()
const { t } = useI18n()
const projects = ref<Project[]>([])
const error = ref('')
const busy = ref(false)
const files = ref<File[]>([])
const c = props.contract
const f = reactive({
  project_id: c?.project_id ?? props.project ?? '', title: c?.title ?? '', counterparty: c?.counterparty ?? '', kind: c?.kind ?? 'sales',
  currency: c?.currency ?? currentMembership()?.currency ?? 'CNY', amount: c ? String(toMajor(c.amount_minor, c.currency)) : '', owner_id: c?.owner_id ?? me.value?.user_id ?? '',
  starts_on: c?.starts_on ?? '', ends_on: c?.ends_on ?? '', summary: c?.summary ?? '',
})
onMounted(async () => {
  projects.value = await api.get<Project[]>('/projects').catch(() => [])
  const p = projects.value.find((x) => x.id === f.project_id)
  if (p && !c) f.currency = p.currency
})
async function save(submit: boolean) {
  error.value = ''
  busy.value = true
  try {
    const body = {
      project_id: f.project_id || null, title: f.title, counterparty: f.counterparty, kind: f.kind, currency: f.currency,
      amount_minor: f.amount ? toMinor(f.amount, f.currency) : 0, owner_id: f.owner_id || null,
      starts_on: f.starts_on || null, ends_on: f.ends_on || null, summary: f.summary, submit: false,
    }
    const saved = c ? await api.put<Contract>(`/contracts/${c.id}`, body) : await api.post<Contract>('/contracts', body)
    if (files.value.length) await uploadDocs(`contracts/${saved.id}`, 'draft', files.value)
    if (submit) await api.post(`/contracts/${saved.id}/submit`)
    emit('saved', saved.id)
  } catch (e) {
    error.value = e instanceof Error && !('status' in e) ? t('common.error', { msg: e.message }) : errorText(e, t)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <form class="card stack" @submit.prevent="save(true)">
    <h2>{{ c ? `${c.key} ${t('lc.edit')}` : t('lc.newContract') }}</h2>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <div class="row">
      <label class="field grow">{{ t('lc.contractTitle') }}<input v-model="f.title" required maxlength="300" /></label>
      <label class="field">{{ t('lc.kind') }}<select v-model="f.kind"><option v-for="k in CONTRACT_KINDS" :key="k" :value="k">{{ t(`lc.ck_${k}`) }}</option></select></label>
    </div>
    <div class="row">
      <label class="field grow">{{ t('lc.counterparty') }}<input v-model="f.counterparty" required maxlength="300" /></label>
      <label class="field grow">{{ t('lc.project') }}<select v-model="f.project_id"><option value="">{{ t('lc.noProject') }}</option><option v-for="p in projects" :key="p.id" :value="p.id">{{ p.key }} {{ p.name }}</option></select></label>
      <label class="field">{{ t('lc.owner') }}<select v-model="f.owner_id"><option v-for="m in members.filter((x) => x.status === 'active')" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select></label>
    </div>
    <div class="row">
      <label class="field" style="flex: 0 1 110px">{{ t('auth.currency') }}<input v-model="f.currency" required maxlength="3" pattern="[A-Z]{3}" @input="f.currency = f.currency.toUpperCase()" /></label>
      <label class="field">{{ t('lc.contractAmount') }}<input v-model="f.amount" type="number" min="0" step="0.01" /></label>
      <label class="field">{{ t('lc.starts') }}<input v-model="f.starts_on" type="date" /></label>
      <label class="field">{{ t('lc.ends') }}<input v-model="f.ends_on" type="date" :min="f.starts_on" /></label>
    </div>
    <label class="field">{{ t('lc.summary') }}<textarea v-model="f.summary" rows="3" maxlength="5000" :placeholder="t('lc.summaryPh')" /></label>
    <label class="field">{{ t('lc.attachDraft') }}<input type="file" multiple @change="files = Array.from(($event.target as HTMLInputElement).files ?? [])" /></label>
    <div class="row">
      <button class="primary" type="submit" :disabled="busy">{{ t('lc.submitContract') }}</button>
      <button type="button" :disabled="busy" @click="save(false)">{{ t('lc.saveDraft') }}</button>
      <button type="button" @click="emit('cancel')">{{ t('common.cancel') }}</button>
    </div>
  </form>
</template>

<style scoped>
.grow { flex: 1 1 200px; }
textarea { padding: 8px 12px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); font: inherit; resize: vertical; }
</style>
