<script setup lang="ts">
// Approval steps, history and (when it is the viewer's turn) the decision form.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Member } from '../../lib/api'
import type { Action, ApprovalView } from '../../lib/lifecycle'

const props = defineProps<{ approval: ApprovalView | null; actions: Action[]; myTurn: boolean; members: Member[]; me: string }>()
const emit = defineEmits<{ decide: [body: { action: string; to_user?: string; note: string }] }>()
const { t, locale } = useI18n()
const fmt = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }))
const note = ref('')
const pass = ref<'delegate' | 'forward' | ''>('')
const to = ref('')
const others = computed(() => props.members.filter((m) => m.status === 'active' && m.user_id !== props.me))
function send(action: string) {
  emit('decide', { action, note: note.value, ...(action === 'delegate' || action === 'forward' ? { to_user: to.value } : {}) })
  note.value = ''
  pass.value = ''
}
const stepName = (s: ApprovalView['steps'][number]) => s.label || t(`lc.step_${s.kind}`)
</script>

<template>
  <section class="stack">
    <ol v-if="approval" class="steps" :aria-label="t('lc.route')">
      <li v-for="s in approval.steps" :key="s.index" :class="{ now: approval.state === 'pending' && s.index === approval.current_step, done: approval.state === 'approved' || s.index < approval.current_step, skipped: s.skipped }">
        <strong>{{ stepName(s) }}</strong>
        <span class="muted small">{{ s.skipped ? t('lc.skipped') : s.users.map((u) => u.name).join('、') || '—' }}</span>
      </li>
    </ol>
    <div v-if="myTurn" class="card inset stack">
      <strong>{{ t('lc.yourTurn') }}</strong>
      <label class="field">{{ t('lc.note') }}<textarea v-model="note" rows="2" maxlength="2000" :placeholder="t('lc.notePh')" /></label>
      <div class="row">
        <button class="primary" type="button" @click="send('approve')">{{ t('lc.approve') }}</button>
        <button type="button" :disabled="!note.trim()" :title="t('lc.reasonNeeded')" @click="send('return')">{{ t('lc.return') }}</button>
        <button type="button" :disabled="!note.trim()" :title="t('lc.reasonNeeded')" @click="send('reject')">{{ t('lc.reject') }}</button>
        <button type="button" :aria-pressed="pass === 'forward'" @click="pass = pass === 'forward' ? '' : 'forward'">{{ t('lc.forward') }}…</button>
        <button type="button" :aria-pressed="pass === 'delegate'" @click="pass = pass === 'delegate' ? '' : 'delegate'">{{ t('lc.delegate') }}…</button>
      </div>
      <div v-if="pass" class="row">
        <label class="field grow">{{ pass === 'forward' ? t('lc.forwardTo') : t('lc.delegateTo') }}
          <select v-model="to"><option value="" disabled>{{ t('org.choose') }}</option><option v-for="m in others" :key="m.user_id" :value="m.user_id">{{ m.display_name }}</option></select>
        </label>
        <button class="primary" type="button" :disabled="!to" style="align-self: flex-end" @click="send(pass)">{{ t('flow.send') }}</button>
      </div>
      <p class="muted small">{{ t('lc.decideHint') }}</p>
    </div>
    <ul class="trail">
      <li v-for="a in actions" :key="a.id">
        <span class="muted small">{{ fmt.format(new Date(a.created_at)) }}</span>
        <span><strong>{{ a.actor_name }}</strong> {{ t(`lc.a_${a.action}`, { to: a.to_name ?? '' }) }}<template v-if="a.note"> · <q>{{ a.note }}</q></template></span>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.steps { display: flex; flex-wrap: wrap; gap: 8px; list-style: none; padding: 0; margin: 0; counter-reset: s; }
.steps li { display: flex; flex-direction: column; padding: 8px 12px; border: 1px solid var(--line); border-radius: 8px; min-width: 120px; background: var(--surface); }
.steps li.done { border-color: var(--accent); background: var(--accent-soft); }
.steps li.now { border-color: var(--warn); background: var(--warn-bg); }
.steps li.skipped { opacity: .65; border-style: dashed; }
.inset { background: var(--inset); padding: 14px; }
textarea { padding: 8px 12px; border: 1px solid var(--line-strong); border-radius: 8px; background: var(--surface); color: var(--text); font: inherit; resize: vertical; }
.trail { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.trail li { display: grid; grid-template-columns: 110px 1fr; gap: 8px; }
.grow { flex: 1 1 200px; }
q { color: var(--text-2); }
@media (max-width: 640px) { .trail li { grid-template-columns: 1fr; gap: 0; } }
</style>
