<script setup lang="ts">
// Project Gantt: task list on the left, timeline on the right. Managers drag a
// bar to move it or drag either end to change start/end; dates snap to days.
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Dependency, Item } from '../lib/api'
import { fromDay, toDay, todayDay, weekday, hours } from '../lib/dates'

const props = defineProps<{ items: Item[]; dependencies: Dependency[]; canManage: boolean; scale: 'day' | 'week' }>()
const emit = defineEmits<{ open: [item: Item]; reschedule: [item: Item, start: string, end: string] }>()
const { t, locale } = useI18n()

const ROW = 44
const dayW = computed(() => (props.scale === 'day' ? 32 : 12))

/** Items in plan order: each phase followed by its children, depth for indentation. */
const rows = computed(() => {
  const out: { item: Item; depth: number }[] = []
  const byParent = new Map<string | null, Item[]>()
  for (const it of props.items) {
    const k = it.parent_id
    if (!byParent.has(k)) byParent.set(k, [])
    byParent.get(k)!.push(it)
  }
  const walk = (p: string | null, depth: number) => {
    for (const it of (byParent.get(p) ?? []).sort((a, b) => a.sort_order - b.sort_order)) {
      out.push({ item: it, depth })
      walk(it.id, depth + 1)
    }
  }
  walk(null, 0)
  return out
})

const range = computed(() => {
  const days = props.items.flatMap((i) => [i.planned_start, i.planned_end]).filter((d): d is string => !!d).map(toDay)
  const today = todayDay()
  let lo = Math.min(today, ...days) - 3
  let hi = Math.max(today + 14, ...days) + 7
  lo -= weekday(lo) // start on a Monday
  hi = Math.max(hi, lo + 28)
  hi += 6 - weekday(hi)
  return { lo, hi, n: hi - lo + 1 }
})

const weeks = computed(() => {
  const out: { label: string; x: number }[] = []
  const fmt = new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', timeZone: 'UTC' })
  for (let d = range.value.lo; d <= range.value.hi; d += 7) out.push({ label: fmt.format(new Date(d * 86_400_000)), x: (d - range.value.lo) * dayW.value })
  return out
})
const weekends = computed(() => {
  const out: number[] = []
  for (let d = range.value.lo; d <= range.value.hi; d++) if (weekday(d) >= 5) out.push((d - range.value.lo) * dayW.value)
  return out
})
const width = computed(() => range.value.n * dayW.value)
const todayX = computed(() => (todayDay() - range.value.lo) * dayW.value)

// Open with today in view (a third of the way across the timeline).
const box = ref<HTMLElement | null>(null)
function scrollToToday() {
  const el = box.value
  if (!el) return
  const timeline = el.clientWidth - 320
  el.scrollLeft = Math.max(0, todayX.value - timeline / 3)
}
onMounted(() => nextTick(scrollToToday))
watch(() => props.scale, () => nextTick(scrollToToday))

// --- dragging ---
type Mode = 'move' | 'start' | 'end'
const drag = ref<{ id: string; mode: Mode; x0: number; s0: number; e0: number; ds: number; de: number } | null>(null)

function span(it: Item): { s: number; e: number } | null {
  if (!it.planned_start && !it.planned_end) return null
  const s = toDay(it.planned_start ?? it.planned_end!)
  const e = toDay(it.planned_end ?? it.planned_start!)
  if (drag.value?.id === it.id) return { s: s + drag.value.ds, e: e + drag.value.de }
  return { s, e }
}

function onDown(ev: PointerEvent, it: Item, mode: Mode) {
  if (!props.canManage || ev.button !== 0) return
  const sp = span(it)
  if (!sp) return
  ev.preventDefault()
  ev.stopPropagation()
  ;(ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId)
  drag.value = { id: it.id, mode: it.kind === 'milestone' ? 'move' : mode, x0: ev.clientX, s0: sp.s, e0: sp.e, ds: 0, de: 0 }
}
function onMove(ev: PointerEvent) {
  const d = drag.value
  if (!d) return
  const delta = Math.round((ev.clientX - d.x0) / dayW.value)
  if (d.mode === 'move') { d.ds = delta; d.de = delta }
  else if (d.mode === 'start') { d.ds = Math.min(delta, d.e0 - d.s0); d.de = 0 }
  else { d.de = Math.max(delta, d.s0 - d.e0); d.ds = 0 }
}
let moved = false
function onUp(it: Item) {
  const d = drag.value
  drag.value = null
  if (!d) return
  moved = d.ds !== 0 || d.de !== 0
  if (moved) emit('reschedule', it, fromDay(d.s0 + d.ds), fromDay(d.e0 + d.de))
}
function onBarClick(it: Item) {
  if (moved) { moved = false; return }
  emit('open', it)
}

function barStyle(it: Item) {
  const sp = span(it)!
  const left = (sp.s - range.value.lo) * dayW.value
  const w = (sp.e - sp.s + 1) * dayW.value
  return { left: `${left}px`, width: `${w}px` }
}
function barClass(it: Item) {
  return ['bar', `k-${it.kind}`, `s-${it.status}`, { dragging: drag.value?.id === it.id, movable: props.canManage }]
}

/** Dependency arrows: from the end of one bar to the start of the next. */
const arrows = computed(() => {
  const index = new Map(rows.value.map((r, i) => [r.item.id, i]))
  const byId = new Map(props.items.map((i) => [i.id, i]))
  return props.dependencies.flatMap((d) => {
    const a = byId.get(d.from_item), b = byId.get(d.to_item)
    const ia = index.get(d.from_item), ib = index.get(d.to_item)
    if (!a || !b || ia === undefined || ib === undefined) return []
    const sa = span(a), sb = span(b)
    if (!sa || !sb) return []
    const x1 = (sa.e - range.value.lo + 1) * dayW.value
    const y1 = ia * ROW + ROW / 2
    const x2 = (sb.s - range.value.lo) * dayW.value
    const y2 = ib * ROW + ROW / 2
    const mid = Math.max(x1 + 8, Math.min(x2 - 8, x1 + 8))
    return [{ key: `${d.from_item}-${d.to_item}`, path: `M${x1} ${y1} H${mid} V${y2} H${x2 - 2}` }]
  })
})
</script>

<template>
  <div ref="box" class="gantt" role="region" :aria-label="t('work.gantt')">
    <div class="grid" :style="{ '--w': `${width}px` }">
      <div class="head list-col">{{ t('work.title') }} · {{ t('work.holder') }}</div>
      <div class="head time-col" :style="{ width: `${width}px` }">
        <span v-for="w in weeks" :key="w.x" class="week" :style="{ left: `${w.x}px` }">{{ w.label }}</span>
      </div>

      <template v-for="(r, i) in rows" :key="r.item.id">
        <button type="button" class="list-col cell" :class="{ phase: r.item.kind === 'phase' }" :style="{ paddingLeft: `${12 + r.depth * 18}px` }" @click="emit('open', r.item)">
          <span class="key">{{ r.item.key }}</span>
          <span class="ttl">{{ r.item.title }}</span>
          <span class="who">{{ r.item.holder_name ?? '' }}</span>
        </button>
        <div class="time-col cell" :style="{ width: `${width}px` }" :data-row="i">
          <template v-if="span(r.item)">
            <div
              :class="barClass(r.item)"
              :style="barStyle(r.item)"
              :title="`${r.item.key} ${r.item.title}`"
              @pointerdown="onDown($event, r.item, 'move')"
              @pointermove="onMove"
              @pointerup="onUp(r.item)"
              @click="onBarClick(r.item)"
            >
              <span v-if="r.item.kind === 'task'" class="fill" :style="{ width: `${r.item.progress_pct}%` }" />
              <span v-if="r.item.kind === 'task'" class="lbl">{{ r.item.progress_pct }}% · {{ hours(r.item.logged_minutes) }}{{ t('work.hours') }}</span>
              <template v-if="canManage && r.item.kind !== 'milestone'">
                <span class="handle l" @pointerdown="onDown($event, r.item, 'start')" @pointermove="onMove" @pointerup="onUp(r.item)" />
                <span class="handle r" @pointerdown="onDown($event, r.item, 'end')" @pointermove="onMove" @pointerup="onUp(r.item)" />
              </template>
            </div>
          </template>
          <span v-else class="unsched">{{ t('work.unscheduled') }}</span>
        </div>
      </template>

      <div class="overlay" aria-hidden="true" :style="{ width: `${width}px`, height: `${rows.length * ROW}px` }">
        <span v-for="x in weekends" :key="x" class="weekend" :style="{ left: `${x}px`, width: `${dayW}px` }" />
        <span class="today" :style="{ left: `${todayX + dayW / 2}px` }" :title="t('work.today')" />
        <svg :width="width" :height="rows.length * ROW">
          <defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 8 4 0 8z" class="arrow-head" /></marker></defs>
          <path v-for="a in arrows" :key="a.key" :d="a.path" class="arrow" marker-end="url(#arrow)" />
        </svg>
      </div>
    </div>
  </div>
</template>

<style scoped>
.gantt { overflow-x: auto; background: var(--surface); border: 1px solid var(--line); border-radius: 12px; }
.grid { display: grid; grid-template-columns: 320px var(--w); position: relative; }
.head { position: sticky; top: 0; background: var(--inset); height: 38px; border-bottom: 1px solid var(--line); font-size: 13px; color: var(--text-3); }
.head.list-col { display: flex; align-items: center; padding: 0 12px; position: sticky; left: 0; z-index: 3; }
.time-col.head { position: relative; }
.week { position: absolute; top: 0; height: 38px; line-height: 38px; padding-left: 6px; border-left: 1px solid var(--line); white-space: nowrap; }
.cell { height: 44px; border-bottom: 1px solid var(--inset); }
.list-col.cell { position: sticky; left: 0; z-index: 2; background: var(--surface); display: flex; align-items: center; gap: 8px; border: 0; border-bottom: 1px solid var(--inset); border-radius: 0; border-right: 1px solid var(--line); text-align: left; padding-right: 12px; color: var(--text); width: 320px; }
.list-col.cell:hover { background: var(--accent-soft); }
.list-col.phase { font-weight: 700; }
.key { font-size: 12px; color: var(--text-3); flex: 0 0 auto; }
.ttl { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.who { font-size: 12px; color: var(--text-2); flex: 0 0 auto; max-width: 80px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.time-col.cell { position: relative; z-index: 1; }
.overlay { position: absolute; left: 320px; top: 38px; pointer-events: none; z-index: 0; }
.overlay svg { position: absolute; inset: 0; overflow: visible; z-index: 1; }
.weekend { position: absolute; top: 0; bottom: 0; background: var(--inset); opacity: .6; }
.today { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--danger); z-index: 2; }
.arrow { fill: none; stroke: var(--text-3); stroke-width: 1.5; }
.arrow-head { fill: var(--text-3); }
.bar { position: absolute; top: 10px; height: 24px; border-radius: 6px; display: flex; align-items: center; overflow: hidden; box-sizing: border-box; user-select: none; touch-action: none; z-index: 2; }
.bar.movable { cursor: grab; }
.bar.dragging { cursor: grabbing; box-shadow: 0 0 0 2px var(--accent); }
.bar.k-phase { top: 18px; height: 8px; border-radius: 2px; background: var(--text-3); }
.bar.k-milestone { top: 13px; width: 18px !important; height: 18px; transform: translateX(7px) rotate(45deg); border-radius: 3px; background: var(--info); }
.bar.k-task { border: 1px solid var(--accent); background: var(--accent-soft); color: var(--text); }
.bar.k-task.s-todo { border-style: dashed; }
.bar.k-task.s-done { border-color: var(--line-strong); background: var(--line); }
.bar.k-task.s-blocked { border-color: var(--danger); }
.fill { position: absolute; left: 0; top: 0; bottom: 0; background: #7BC092; opacity: .9; }
:root[data-theme="dark"] .fill { background: #2E5D4A; }
.lbl { position: relative; font-size: 12px; padding: 0 8px; white-space: nowrap; }
.handle { position: absolute; top: 0; bottom: 0; width: 8px; cursor: ew-resize; }
.handle.l { left: 0; }
.handle.r { right: 0; }
.unsched { position: absolute; left: 8px; top: 12px; font-size: 12px; color: var(--text-3); }
</style>
