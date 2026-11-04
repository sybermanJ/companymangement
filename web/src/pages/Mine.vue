<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Shell from './Shell.vue'
import ItemPanel from './ItemPanel.vue'
import { api, type Item, type Member, type Timer } from '../lib/api'
import { errorText } from '../lib/errors'
import { hours } from '../lib/dates'

const { t } = useI18n()
const items = ref<Item[]>([])
const timer = ref<Timer | null>(null)
const members = ref<Member[]>([])
const open = ref<string | null>(null)
const error = ref('')

async function load() {
  const d = await api.get<{ items: Item[]; timer: Timer | null }>('/my/items')
  items.value = d.items
  timer.value = d.timer
}
onMounted(async () => {
  try {
    await load()
    members.value = await api.get<Member[]>('/members')
  } catch (e) {
    error.value = errorText(e, t)
  }
})
const running = computed(() => items.value.find((i) => i.id === timer.value?.work_item_id))
</script>

<template>
  <Shell>
    <div class="stack">
      <h1>{{ t('work.mine') }}</h1>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="running" class="notice ok">{{ t('work.timerRunning', { key: running.key, title: running.title }) }}</p>
      <p v-if="!items.length && !error" class="muted">{{ t('work.noMine') }}</p>
      <div class="layout" :class="{ withPanel: !!open }">
        <ul class="list">
          <li v-for="i in items" :key="i.id">
            <button type="button" class="card itemrow" :class="{ on: open === i.id }" @click="open = i.id">
              <span class="muted small">{{ i.key }}</span>
              <span class="ttl">{{ i.title }}</span>
              <span class="badge">{{ t(`work.s_${i.status}`) }}</span>
              <span class="muted small">{{ i.planned_end ?? '' }}</span>
              <span class="muted small">{{ hours(i.logged_minutes) }}{{ t('work.hours') }}</span>
            </button>
          </li>
        </ul>
        <ItemPanel v-if="open" :item-id="open" :members="members" @close="open = null" @changed="load" />
      </div>
    </div>
  </Shell>
</template>

<style scoped>
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
.itemrow { width: 100%; height: auto; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; text-align: left; padding: 12px 16px; }
.itemrow.on { border-color: var(--accent); }
.ttl { flex: 1 1 200px; min-width: 0; overflow-wrap: anywhere; }
.small { font-size: 12px; }
.layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
.layout.withPanel { grid-template-columns: minmax(0, 1fr) minmax(320px, 440px); }
@media (max-width: 1000px) { .layout.withPanel { grid-template-columns: minmax(0, 1fr); } }
</style>
