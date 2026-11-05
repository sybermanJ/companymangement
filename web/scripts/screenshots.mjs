// Seeds a demo company through the API, then captures real screens and checks
// for horizontal overflow. Usage: node web/scripts/screenshots.mjs [baseUrl] [outDir]
// Needs a running cm-server with the built web app, Playwright, and psql with
// DATABASE_URL set (used only to read invitation links from the email outbox).
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { createHmac } from 'node:crypto'
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
const tester = await call('POST', '/roles', { name: '测试工程师', department_id: qa.id })
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

// --- Routine work, requests, hand-offs (phase 2) ---
await call('POST', '/role-assignments', { user_id: people.zhaomin.id, role_id: tester.id, department_id: qa.id })
await call('POST', '/role-assignments', { user_id: people.wanglei.id, role_id: be.id, department_id: rd.id })
await call('PUT', `/departments/${qa.id}/head`, { head_user_id: me.user_id })
await call('POST', '/routines', {
  title: '每日冒烟测试', description: '主流程下单、支付、退款各走一遍', recurrence: 'daily', weekdays: [],
  target_role_id: tester.id, target_department_id: qa.id, mode: 'each_member', estimate_minutes: 60, project_id: proj.id,
  deliverables: [{ kind: 'text', min_count: 1, required: true, instructions: '结论：通过/不通过' }, { kind: 'image', min_count: 2, required: true, instructions: '关键页面截图' }],
})
await call('POST', '/routines', {
  title: '工作日报', recurrence: 'weekdays', weekdays: [], target_user_id: people.wanglei.id, mode: 'each_member', estimate_minutes: 15,
  deliverables: [{ kind: 'text', min_count: 1, required: true, instructions: '今天完成了什么，明天计划' }],
})
await call('POST', '/routines', {
  title: '周例会纪要', recurrence: 'weekly', weekdays: [1], target_role_id: be.id, target_department_id: rd.id, mode: 'pool',
  deliverables: [{ kind: 'text', min_count: 1, required: true }, { kind: 'link', min_count: 1, required: false, instructions: '会议录像链接' }],
})
await call('POST', '/routines/generate', {})
await call('POST', '/requests', { to_user: people.wanglei.id, title: '帮忙看一下退款回调的报错日志', description: '线上昨晚 3 笔退款回调 500', due: d(1), project_id: proj.id, estimate_minutes: 60 })
// Zhao Min (QA) hands the reconciliation task to Wang Lei (R&D): cross-department, needs approval.
const t4now = await zc('GET', `/items/${t4.id}`)
await zc('POST', `/items/${t4.id}/handoffs`, { version: t4now.item.version, to_user: people.wanglei.id, reason: '对账逻辑和订单服务耦合，交给后端更合适', new_due: d(6) })
// Wang Lei submits his daily report deliverable.
const wlMine = await wl('GET', '/my/items')
const report = (wlMine.items ?? []).find((i) => i.title === '工作日报')
if (report) {
  const det = await wl('GET', `/items/${report.id}`)
  await wl('POST', `/items/${report.id}/submissions`, { spec_id: det.specs[0].id, value: '完成订单幂等改造；明天做回调重试。' })
}

// --- Finance (phase 3): rates in two currencies, FX, thresholds ---
for (const [u, minor, ccy] of [[me.user_id, 8000, 'USD'], [people.wanglei.id, 6000, 'USD'], [people.zhaomin.id, 30000, 'CNY']]) {
  await call('POST', '/finance/rates', { user_id: u, rate_minor: minor, currency: ccy, effective_from: d(-60), note: '2026 年度费率' })
}
await call('POST', '/finance/fx', { from_ccy: 'CNY', to_ccy: 'USD', rate: '0.1400', effective_date: d(-60) })
await call('POST', `/finance/projects/${proj.id}/thresholds`, { mode: 'percent', percent: 50, level: 'warn' })
await call('POST', `/finance/projects/${proj.id}/thresholds`, { mode: 'percent', percent: 80, level: 'critical', notify_admins: true })
await call('POST', `/finance/projects/${proj.id}/thresholds`, { mode: 'absolute', amount_minor: 250000, level: 'info' })
await call('POST', `/finance/projects/${proj.id}/thresholds`, { metric: 'forecast', mode: 'percent', percent: 100, level: 'critical' })

// --- Git (phase 4): a self-hosted Gitea repo pushing a commit that names an item ---
const repo = await call('POST', '/git/repos', { provider: 'gitea', name: 'pay/order-service', url: 'https://git.example.test/pay/order-service', project_id: proj.id })
await call('POST', '/git/identities', { email: 'wanglei@dev.example', user_id: people.wanglei.id })
const t3now = await call('GET', `/items/${t3.id}`)
const hookBody = JSON.stringify({ ref: `refs/heads/feature/${t3now.item.key}-idempotency`, commits: [
  { id: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678', message: `${t3now.item.key} 下单接口幂等`, url: 'https://git.example.test/pay/order-service/commit/a1b2c3d', timestamp: new Date().toISOString(), author: { name: '王磊', email: 'wanglei@dev.example' } },
  { id: 'b2c3d4e5f60718293a4b5c6d7e8f901234567890', message: '重复回调单元测试', url: 'https://git.example.test/pay/order-service/commit/b2c3d4e', timestamp: new Date().toISOString(), author: { name: '外包同事', email: 'contractor@vendor.example' } },
] })
const hook = await fetch(`${base}/api/v1/hooks/git/${repo.repo.id}`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-gitea-event': 'push', 'x-gitea-signature': createHmac('sha256', repo.secret).update(hookBody).digest('hex') }, body: hookBody })
if (!hook.ok) { console.error('webhook failed', hook.status, await hook.text()); process.exit(1) }
console.log('webhook', await hook.json())

// --- Project lifecycle (phase 6): requests, trip with delegated booking, contract, routes ---
await call('PUT', `/projects/${proj.id}/expense-budget`, { budget_minor: 300000 })
await call('PUT', `/phases/${p2.id}/budget`, { budget_minor: 150000 })
await call('POST', `/finance/projects/${proj.id}/thresholds`, { metric: 'expense', mode: 'percent', percent: 80, level: 'warn' })
const reim = await wl('POST', '/claims', { kind: 'reimbursement', project_id: proj.id, phase_id: p1.id, title: '客户现场需求调研差旅', justification: '客户要求到现场梳理对账流程，与财务、运营两个部门开了两天工作坊。', currency: 'CNY', lines: [
  { spent_on: d(-15), category: 'flight', description: '上海 → 成都 往返', amount_minor: 248000 },
  { spent_on: d(-15), category: 'hotel', description: '2 晚', amount_minor: 96000 },
  { spent_on: d(-14), category: 'local_transport', description: '打车', amount_minor: 12600 },
  { spent_on: d(-14), category: 'meals', description: '工作餐', amount_minor: 18000 }], submit: true })
await p2page.evaluate(async ([id]) => {
  const f = new FormData(); f.append('label', 'invoice'); f.append('file', new Blob(['%PDF-1.4 demo'], { type: 'application/pdf' }), '机票行程单.pdf')
  await fetch(`/api/v1/claims/${id}/documents`, { method: 'POST', body: f, headers: { 'X-Requested-With': 'cm' } })
}, [reim.id])
await call('POST', `/claims/${reim.id}/decide`, { action: 'approve', note: '与需求阶段计划一致' })
await call('POST', `/claims/${reim.id}/decide`, { action: 'approve' })
await call('POST', `/claims/${reim.id}/pay`, { reference: 'CMB-20261105-0032' })
await wl('POST', `/claims/${reim.id}/close`, {})
const trip = await wl('POST', '/claims', { kind: 'trip', project_id: proj.id, phase_id: p3.id, title: '上线前现场联调', justification: '上线前需要在客户机房完成支付网关联调和压测。', currency: 'CNY', destination: '成都', depart_on: d(6), return_on: d(9), travellers: [people.wanglei.id, people.zhaomin.id], lines: [{ category: 'flight', amount_minor: 420000 }, { category: 'hotel', amount_minor: 180000 }, { category: 'allowance', amount_minor: 48000 }], submit: true })
await call('POST', `/claims/${trip.id}/decide`, { action: 'approve', note: '同意，注意控制住宿标准' })
await wl('POST', `/claims/${trip.id}/bookings`, { to_user: people.zhaomin.id, title: `订 ${d(6)} 上海 → 成都机票 2 张`, due_on: d(2) })
await wl('POST', '/claims', { kind: 'purchase', project_id: proj.id, phase_id: p2.id, title: '压测用云服务器（1 个月）', justification: '对账模块需要在接近生产规模的数据上压测。', currency: 'CNY', payee: '某云计算公司', lines: [{ category: 'software', description: '8 核 32G × 3 台', amount_minor: 360000 }], submit: true })
await zc('POST', '/claims', { kind: 'reimbursement', project_id: proj.id, phase_id: p2.id, title: '对账联调加班餐费', justification: '周末与银行联调对账文件。', currency: 'CNY', lines: [{ spent_on: d(-3), category: 'meals', amount_minor: 23600 }], submit: true })
await call('POST', '/approval-routes', { subject: 'payment', name: '大额付款', min_amount_minor: 5000000, steps: [{ kind: 'project_manager' }, { kind: 'finance' }, { kind: 'admin', label: '总经理' }] })
const ct = await call('POST', '/contracts', { project_id: proj.id, title: '支付系统重构开发合同', counterparty: '某连锁零售集团', kind: 'sales', currency: 'USD', amount_minor: 18000000, starts_on: d(-20), ends_on: d(60), summary: '分三期付款：签约 30%，联调完成 40%，上线验收 30%。', submit: true })
await call('POST', `/contracts/${ct.id}/status`, { status: 'signed', signed_on: d(-18) })
for (const [ph, title, due, amt] of [[p1, '签约款 30%', d(-18), 5400000], [p2, '联调完成 40%', d(5), 7200000], [p3, '上线验收 30%', d(14), 5400000]]) {
  const m = await call('POST', `/contracts/${ct.id}/milestones`, { phase_id: ph.id, title, due_on: due, amount_minor: amt })
  if (title.startsWith('签约')) await call('POST', `/milestones/${m.id}/status`, { status: 'settled', settled_on: d(-10) })
}
await page.evaluate(async ([id]) => {
  for (const [label, name] of [['draft', '开发合同-草稿.docx'], ['signed', '开发合同-签署版.pdf']]) {
    const f = new FormData(); f.append('label', label); f.append('file', new Blob(['demo'], { type: 'application/pdf' }), name)
    await fetch(`/api/v1/contracts/${id}/documents`, { method: 'POST', body: f, headers: { 'X-Requested-With': 'cm' } })
  }
}, [ct.id])
await call('POST', '/contracts', { project_id: proj.id, title: '对账模块外包协议', counterparty: '某软件工作室', kind: 'outsourcing', currency: 'CNY', amount_minor: 8000000, summary: '外包对账报表开发，按验收付款。' })
const pending = (await call('GET', '/claims?view=to_approve')).find((c) => c.kind === 'purchase')

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
await bar.scrollIntoViewIfNeeded()
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
await page.setViewportSize({ width: 1280, height: 800 })
await page.locator('#lang').selectOption('zh-CN')
await page.evaluate(() => { document.documentElement.dataset.theme = 'light' })
await shot('overview-light', async () => { await page.goto(base + '/overview') })
await shot('team-light', async () => { await page.goto(base + '/team'); await page.getByRole('button', { name: '王磊' }).click(); await page.waitForTimeout(500) })
await shot('requests-approve-light', async () => { await page.goto(base + `/requests?view=to_approve&id=${pending.id}`); await page.waitForTimeout(500) })
await shot('requests-all-light', async () => { await page.goto(base + `/requests?view=all&id=${trip.id}`); await page.waitForTimeout(500) })
await shot('request-new-trip-light', async () => { await p2page.goto(base + '/requests?new=trip'); await p2page.waitForTimeout(500) }, p2page)
await shot('contracts-light', async () => { await page.goto(base + `/contracts?id=${ct.id}`); await page.waitForTimeout(500) })
await shot('project-file-light', async () => { await page.goto(base + `/projects/${proj.id}?tab=file`); await page.waitForTimeout(600) })
await shot('org-routes-light', async () => { await page.goto(base + '/org'); await page.getByRole('tab', { name: '审批流程' }).click(); await page.waitForTimeout(300) })
await shot('project-file-en-dark', async () => {
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark' })
  await page.locator('#lang').selectOption('en')
  await page.goto(base + `/projects/${proj.id}?tab=file`); await page.waitForTimeout(600)
})
await page.locator('#lang').selectOption('zh-CN')
await page.evaluate(() => { document.documentElement.dataset.theme = 'light' })
await page.setViewportSize({ width: 390, height: 844 })
await shot('requests-phone-ja', async () => { await page.locator('#lang').selectOption('ja'); await page.goto(base + `/requests?view=to_approve&id=${pending.id}`); await page.waitForTimeout(500) })
await shot('project-file-phone', async () => { await page.locator('#lang').selectOption('zh-CN'); await page.goto(base + `/projects/${proj.id}?tab=file`); await page.waitForTimeout(600) })
await page.setViewportSize({ width: 1280, height: 800 })
await shot('inbox-light', async () => { await page.goto(base + '/inbox') })
await shot('routines-light', async () => { await page.goto(base + '/routines') })
await shot('finance-costs-light', async () => { await page.goto(base + '/finance') })
await shot('finance-rates-dark', async () => {
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark' })
  await page.getByRole('tab').nth(1).click()
})
await page.evaluate(() => { document.documentElement.dataset.theme = 'light' })
await shot('org-git-light', async () => { await page.goto(base + '/org'); await page.getByRole('tab').last().click() })
await shot('my-work-routines-light', async () => {
  await p2page.setViewportSize({ width: 1280, height: 800 })
  await p2page.goto(base + '/my')
}, p2page)
await shot('item-commits-light', async () => {
  await p2page.goto(base + `/my?item=${t3.id}`)
  await p2page.waitForTimeout(600)
}, p2page)
await page.setViewportSize({ width: 390, height: 844 })
await shot('inbox-phone', async () => { await page.goto(base + '/inbox') })
await shot('finance-phone-en', async () => { await page.locator('#lang').selectOption('en'); await page.goto(base + '/finance') })
await page.locator('#lang').selectOption('zh-CN')
await page.evaluate(async () => { await fetch('/api/v1/logout', { method: 'POST', headers: { 'X-Requested-With': 'cm' } }) })
await shot('signup-phone', async () => { await page.goto(base + '/signup') })

console.log(JSON.stringify(shots, null, 2))
const failed = shots.filter((s) => s.scroll > s.client || s.bad.length)
await browser.close()
if (failed.length) { console.error('horizontal overflow on', failed.map((f) => f.name)); process.exit(1) }
