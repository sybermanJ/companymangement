// Seeds a demo company through the API, then captures real screens and checks
// for horizontal overflow. Usage: node web/scripts/screenshots.mjs [baseUrl] [outDir]
// Needs a running cm-server with the built web app and Playwright installed.
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
    return r.status === 204 ? null : r.json()
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
for (const [n, e, fin] of [['王磊', 'wanglei'], ['赵敏', 'zhaomin'], ['孙悦', 'sunyue'], ['周财', 'zhoucai', true]]) {
  await call('POST', '/invitations', { email: `${e}-${tag}@example.test`, name: n, is_finance: !!fin })
}
const be = await call('POST', '/roles', { name: '后端工程师', department_id: rd.id })
await call('POST', '/roles', { name: '测试工程师', department_id: qa.id })
await call('POST', '/roles', { name: '项目经理', department_id: null })
const me = await call('GET', '/me')
await call('POST', '/role-assignments', { user_id: me.user_id, role_id: be.id, department_id: rd.id })

const shots = []
async function shot(name, setup) {
  await setup()
  await page.waitForTimeout(300)
  const overflow = await page.evaluate(() => {
    const doc = document.documentElement
    const bad = [...document.querySelectorAll('body *')].filter((el) => {
      const r = el.getBoundingClientRect()
      return r.right > doc.clientWidth + 1 && !el.closest('.table-wrap')
    }).map((el) => el.tagName + '.' + el.className).slice(0, 5)
    return { scroll: doc.scrollWidth, client: doc.clientWidth, bad }
  })
  shots.push({ name, ...overflow })
  await page.screenshot({ path: `${out}/${name}.png`, fullPage: true })
}

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
await page.evaluate(async () => { await fetch('/api/v1/logout', { method: 'POST', headers: { 'X-Requested-With': 'cm' } }) })
await shot('signup-phone', async () => { await page.goto(base + '/signup') })

console.log(JSON.stringify(shots, null, 2))
const failed = shots.filter((s) => s.scroll > s.client || s.bad.length)
await browser.close()
if (failed.length) { console.error('horizontal overflow on', failed.map((f) => f.name)); process.exit(1) }
