// Seeds a demo company through the API, then captures real screens and checks
// for horizontal overflow. Usage: node web/scripts/screenshots.mjs [baseUrl] [outDir]
// Needs a running cm-server with the built web app, Playwright, and psql with
// DATABASE_URL set (used only to read invitation links from the email outbox).
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
const require = createRequire(import.meta.url)
const { chromium } = require('playwright')

const base = process.argv[2] ?? 'http://127.0.0.1:8080'
const out = process.argv[3] ?? 'docs/screenshots'
const tag = Date.now().toString(36)
const email = `demo-${tag}@example.test`
const password = 'demo password 123'

const browser = await chromium.launch()
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, locale: 'zh-CN' })
const page = await ctx.newPage()
const call = (method, path, body) =>
  page.evaluate(async ([m, p, b]) => {
    const r = await fetch('/api/v1' + p, { method: m, headers: { 'Content-Type': 'application/json', 'X-Requested-With': 'cm' }, body: b ? JSON.stringify(b) : undefined })
    const txt = await r.text(); return txt ? JSON.parse(txt) : null
  }, [method, path, body])

await page.goto(base + '/login')
await page.screenshot({ path: `${out}/login-light.png` })

await call('POST', '/signup', { company_name: '示例科技有限公司', slug: `demo-${tag}`, default_currency: 'CNY', name: '林晓', email, password })
await call('POST', '/login', { email, password })
const depts = await call('GET', '/departments')
const root = depts[0].id
const rd = await call('POST', '/departments', { name: '研发部', parent_id: root })
const qa = await call('POST', '/departments', { name: '质量部', parent_id: root })
await call('POST', '/departments', { name: '产品部', parent_id: root })
await call('POST', '/departments', { name: '后端组', parent_id: rd.id })
const people = {}
for (const [n, e, fin] of [['王磊', 'wanglei'], ['赵敏', 'zhaomin'], ['孙悦', 'sunyue'], ['周财', 'zhoucai', true]]) {
  const addr = `${e}-${tag}@example.test`
  const inv = await call('POST', '/invitations', { email: addr, name: n, is_finance: !!fin })
  people[e] = { id: inv.user_id, email: addr }
}
// Accept two invitations so they can hold work (others stay "invited" for the members screenshot).
function inviteToken(addr) {
  const body = execFileSync('psql', [process.env.DATABASE_URL, '-tA', '-c', `SELECT body_text FROM outbox_email WHERE to_addr = '${addr}' ORDER BY created_at DESC LIMIT 1`]).toString()
  return body.slice(body.indexOf('token=') + 6, body.indexOf('token=') + 70)
}
for (const k of ['wanglei', 'zhaomin']) {
  await call('POST', '/invitations/accept', { token: inviteToken(people[k].email), password: 'member password 1' })
}
const be = await call('POST', '/roles', { name: '后端工程师', department_id: rd.id })
await call('POST', '/roles', { name: '测试工程师', department_id: qa.id })
await call('POST', '/roles', { name: '项目经理', department_id: null })
const me = await call('GET', '/me')
await call('POST', '/role-assignments', { user_id: me.user_id, role_id: be.id, department_id: rd.id })

// A project shaped like the approved mockup, around today's date.
const DAY = 86400000
const today = new Date(Date.UTC(new Date().getFullYear(), new Date().getMonth(), new Date().getDate()))
const d = (n) => new Date(today.getTime() + n * DAY).toISOString().slice(0, 10)
const proj = await call('POST', '/projects', { key: 'PAY', name: '支付系统重构', labour_budget_minor: 6000000, currency: 'USD', planned_start: d(-16), planned_end: d(11) })
const add = (b) => call('POST', `/projects/${proj.id}/items`, b)
const p1 = await add({ kind: 'phase', title: '需求与设计', planned_start: d(-16), planned_end: d(-10) })
const t1 = await add({ kind: 'task', title: '接口设计评审', parent_id: p1.id, holder_user_id: me.user_id, planned_start: d(-16), planned_end: d(-13), estimate_minutes: 960 })
const t2 = await add({ kind: 'task', title: '数据库方案', parent_id: p1.id, holder_user_id: people.zhaomin.id, planned_start: d(-14), planned_end: d(-10), estimate_minutes: 1440 })
const p2 = await add({ kind: 'phase', title: '开发', planned_start: d(-9), planned_end: d(4) })
const t3 = await add({ kind: 'task', title: '订单服务改造', parent_id: p2.id, holder_user_id: people.wanglei.id, planned_start: d(-9), planned_end: d(2), estimate_minutes: 4800 })
const t4 = await add({ kind: 'task', title: '对账模块', parent_id: p2.id, holder_user_id: people.zhaomin.id, planned_start: d(-7), planned_end: d(4), estimate_minutes: 3600 })
const t5 = await add({ kind: 'task', title: '前端结算页', parent_id: p2.id, planned_start: d(1), planned_end: d(6), estimate_minutes: 2400 })
const m1 = await add({ kind: 'milestone', title: '联调开始', planned_start: d(5) })
const p3 = await add({ kind: 'phase', title: '测试与上线', planned_start: d(5), planned_end: d(11) })
const t6 = await add({ kind: 'task', title: '集成测试', parent_id: p3.id, planned_start: d(5), planned_end: d(9), estimate_minutes: 2400 })
await add({ kind: 'milestone', title: '上线', parent_id: p3.id, planned_start: d(11) })
for (const [a, b] of [[t2, t3], [t3, t5], [t4, m1], [m1, t6]]) await call('POST', '/dependencies', { from_item: a.id, to_item: b.id })
for (const [t, v, s, pct] of [[t1, 1, ['in_progress', 'done'], 100], [t2, 1, ['in_progress', 'done'], 100]]) {
  let ver = v
  for (const st of s) { const r = await call('POST', `/items/${t.id}/status`, { version: ver, status: st }); ver = r.version }
}
await call('POST', `/items/${t1.id}/logs`, { minutes: 900, description: '评审会议与接口文档' })
await call('POST', `/items/${t2.id}/logs`, { minutes: 1320, description: '表结构与迁移脚本' })
// Members log their own work.
const ctx2 = await browser.newContext({ deviceScaleFactor: 2, locale: 'zh-CN' })
const p2page = await ctx2.newPage()
await p2page.goto(base + '/login')
const as = (pg) => (method, path, body) => pg.evaluate(async ([m, p, b]) => {
  const r = await fetch('/api/v1' + p, { method: m, headers: { 'Content-Type': 'application/json', 'X-Requested-With': 'cm' }, body: b ? JSON.stringify(b) : undefined })
  const txt = await r.text(); return txt ? JSON.parse(txt) : null
}, [method, path, body])
const wl = as(p2page)
await wl('POST', '/login', { email: people.wanglei.email, password: 'member password 1' })
let st = await wl('POST', `/items/${t3.id}/status`, { version: 1, status: 'in_progress' })
st = await wl('PATCH', `/items/${t3.id}`, { version: st.version, progress_pct: 60 })
const lg = await wl('POST', `/items/${t3.id}/logs`, { minutes: 180, description: '完成下单接口幂等改造，补充重复支付回调的处理和单元测试。' })
await wl('POST', `/items/${t3.id}/logs`, { minutes: 240, description: '订单状态机重构' })
// A small generated PNG as the screenshot attachment.
const png = await p2page.evaluate(async () => {
  const c = document.createElement('canvas'); c.width = 320; c.height = 200
  const g = c.getContext('2d'); g.fillStyle = '#D8E3E7'; g.fillRect(0, 0, 320, 200)
  g.fillStyle = '#2E5D4A'; for (let i = 0; i < 12; i++) g.fillRect(20 + i * 24, 180 - (i * 37 % 140), 16, i * 37 % 140)
  return c.toDataURL('image/png')
})
await p2page.evaluate(async ([item, log, dataUrl]) => {
  const blob = await (await fetch(dataUrl)).blob()
  const f = new FormData(); f.append('work_log_id', log); f.append('file', blob, '接口压测截图.png')
  await fetch(`/api/v1/items/${item}/attachments`, { method: 'POST', body: f, headers: { 'X-Requested-With': 'cm' } })
}, [t3.id, lg.id, png])
const zm = await browser.newContext({ locale: 'zh-CN' }).then((c) => c.newPage())
await zm.goto(base + '/login')
const zc = as(zm)
await zc('POST', '/login', { email: people.zhaomin.email, password: 'member password 1' })
st = await zc('POST', `/items/${t4.id}/status`, { version: 1, status: 'in_progress' })
await zc('PATCH', `/items/${t4.id}`, { version: st.version, progress_pct: 35 })
await zc('POST', `/items/${t4.id}/logs`, { minutes: 300, description: '对账字段梳理' })

const shots = []
async function shot(name, setup, pg = page) {
  await setup()
  await pg.waitForTimeout(300)
  const overflow = await pg.evaluate(() => {
    const doc = document.documentElement
    const bad = [...document.querySelectorAll('body *')].filter((el) => {
      const r = el.getBoundingClientRect()
      return r.right > doc.clientWidth + 1 && !el.closest('.table-wrap, .gantt')
    }).map((el) => el.tagName + '.' + el.className).slice(0, 5)
    return { scroll: doc.scrollWidth, client: doc.clientWidth, bad }
  })
  shots.push({ name, ...overflow })
  await pg.screenshot({ path: `${out}/${name}.png`, fullPage: true })
}

await shot('gantt-light', async () => { await page.goto(base + `/projects/${proj.id}`) })

// Drag the "前端结算页" bar two days to the right and check the API saw it.
const bar = page.locator('.bar.k-task').nth(4)
const box = await bar.boundingBox()
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
await page.mouse.down()
await page.mouse.move(box.x + box.width / 2 + 64, box.y + box.height / 2, { steps: 8 })
await page.mouse.up()
await page.waitForTimeout(500)
const after = await call('GET', `/items/${t5.id}`)
if (after.item.planned_start !== d(3)) { console.error('drag did not reschedule', after.item.planned_start, d(3)); process.exit(1) }
console.log('drag rescheduled', d(1), '->', after.item.planned_start)

await shot('gantt-item-panel-light', async () => {
  await page.getByRole('button', { name: /订单服务改造/ }).first().click()
  await page.waitForTimeout(600)
})
await shot('gantt-week-dark', async () => {
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark' })
  await page.getByRole('button', { name: '关闭' }).click()
  await page.getByRole('button', { name: '按周' }).click()
})
await page.evaluate(() => { document.documentElement.dataset.theme = 'light' })
await shot('projects-light', async () => { await page.goto(base + '/projects') })
await shot('org-departments-light', async () => { await page.goto(base + '/org') })
await shot('org-members-light', async () => { await page.getByRole('tab').nth(1).click() })
await shot('org-roles-dark', async () => {
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark' })
  await page.getByRole('tab').nth(2).click()
})
await shot('org-members-en-dark', async () => {
  await page.locator('#lang').selectOption('en')
  await page.getByRole('tab').nth(1).click()
})
await page.setViewportSize({ width: 390, height: 844 })
await shot('org-departments-phone-ja', async () => {
  await page.locator('#lang').selectOption('ja')
  await page.evaluate(() => { document.documentElement.dataset.theme = 'light' })
  await page.getByRole('tab').nth(0).click()
})
await shot('my-work-phone', async () => {
  await p2page.setViewportSize({ width: 390, height: 844 })
  await p2page.goto(base + '/my')
  await p2page.getByRole('button', { name: /订单服务改造/ }).first().click()
  await p2page.waitForTimeout(600)
}, p2page)
await page.evaluate(async () => { await fetch('/api/v1/logout', { method: 'POST', headers: { 'X-Requested-With': 'cm' } }) })
await shot('signup-phone', async () => { await page.goto(base + '/signup') })

console.log(JSON.stringify(shots, null, 2))
const failed = shots.filter((s) => s.scroll > s.client || s.bad.length)
await browser.close()
if (failed.length) { console.error('horizontal overflow on', failed.map((f) => f.name)); process.exit(1) }
