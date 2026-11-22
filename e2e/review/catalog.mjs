/**
 * Game version and loader pickers, the install plan, consent and deploy
 * progress, on the mocked bridge (fixture mode):
 *   - pick 1.21.4, Forge and a Forge build; the plan preview lists every
 *     step with the installer's full argv;
 *   - a server sandbox: Deploy asks consent (exec statement, trust line,
 *     EULA checkbox gating Install), then shows progress until done;
 *   - an installer that exits non-zero: the failed step is marked and its
 *     log opens;
 *   - Custom… refuses `../x` and takes `1.21.4-custom`;
 *   - a stale catalog shows the offline banner;
 *   - a game with no catalog keeps the free-text fields.
 * Screenshots go to docs/review/catalog-*.png (`-light` with THEME=light).
 * Prints PASS/FAIL per check; exits 1 on any FAIL or page error.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
const theme = process.env.THEME === 'light' ? 'light' : 'dark'
const suffix = theme === 'light' ? '-light' : ''
const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
let failed = 0
const check = (name, ok, detail = '') => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${ok || !detail ? '' : ` (${detail})`}`)
    if (!ok) failed++
}

const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, colorScheme: theme })
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
page.on('dialog', (d) => d.accept())
await page.addInitScript(() => (globalThis.__reviewFixtures = true))

const calls = (c) => page.evaluate((cmd) => globalThis.__reviewLog.filter((x) => x.cmd === cmd), c)
const shot = (name, target = page) => target.screenshot({ path: `${out}catalog-${name}${suffix}.png` })
const button = (re, scope = page) => scope.locator('button:visible', { hasText: re }).first()
const combo = (name) => page.getByRole('combobox', { name, exact: true }).first()
const pick = async (name, option) => {
    await combo(name).click()
    await page.getByRole('option', { name: option }).first().click()
    await page.waitForTimeout(300)
}
const sandbox = (patch) => page.evaluate((p) => Object.assign(globalThis.__reviewSandboxes.get(1), p), patch)
/**
 * Leave the sandboxes screen and come back, so it reads the rows again: a
 * same-document navigation, so the mock keeps its state.
 */
const revisit = async () => {
    await page.goto(base + '/downloads')
    await page.waitForTimeout(300)
    await page.goto(base + '/sandboxes/1')
    await page.getByRole('heading', { name: 'Settings' }).waitFor({ timeout: 10000 })
    await page.waitForTimeout(400)
}
const step = async (name, fn) => {
    try {
        await fn()
    } catch (e) {
        check(name, false, String(e.message ?? e).split('\n')[0])
        await page.screenshot({ path: `${out}catalog-debug-${name.replace(/\W+/g, '-')}.png` }).catch(() => {})
    }
}

await step('setup', async () => {
    await page.goto(base + '/sandboxes')
    await button(/New sandbox/).click()
    await page.getByRole('button', { name: 'Game' }).or(page.getByRole('combobox', { name: 'Game' })).first().click()
    await page.getByRole('option', { name: /Counter-Strike: Source/ }).first().click()
    await page.getByRole('textbox').last().fill('Review catalog')
    await button(/^Create/).click()
    await page.waitForTimeout(600)
})

// ------------------------------------------------------------ No catalog: free text
await step('free text', async () => {
    await sandbox({ gameVersion: null, loader: null, loaderVersion: null, environment: 'client' })
    await revisit()
    await page.getByTestId('catalog-free-text').waitFor({ timeout: 8000 })
    check('free text: a game with no catalog gets the typed fields', await page.getByRole('textbox', { name: 'Game version' }).isVisible())
    await page.getByTestId('catalog-free-text').scrollIntoViewIfNeeded()
    await shot('free-text')
})

// ------------------------------------------------------------ Pickers and the plan preview
await step('pickers', async () => {
    await sandbox({ appSlug: 'minecraft', appName: 'Minecraft' })
    await revisit()
    await combo('Game version').waitFor({ timeout: 8000 })
    await pick('Game version', /^1\.21\.4/)
    check('pick: game version patched to 1.21.4', (await calls('sandbox_patch')).some((c) => c.args.patch?.gameVersion === '1.21.4'))

    await pick('Loader', /^Forge/)
    const lp = (await calls('sandbox_patch')).find((c) => c.args.patch?.loader === 'forge')
    check('pick: Forge patched with its recommended build', lp?.args.patch?.loaderVersion === '54.1.0', JSON.stringify(lp?.args))
    const lv = await calls('catalog_loader_versions')
    check('pick: builds asked for 1.21.4', lv.some((c) => c.args.gameVersion === '1.21.4' && c.args.loader === 'forge'), JSON.stringify(lv.map((c) => c.args)))

    await pick('Loader version', /^54\.0\.26/)
    check('pick: loader version patched', (await calls('sandbox_patch')).some((c) => c.args.patch?.loaderVersion === '54.0.26'))
    await shot('pickers')

    const plan = page.getByTestId('install-plan')
    await plan.locator('summary').first().click()
    await plan.getByTestId('plan-step').first().waitFor({ timeout: 5000 })
    const n = await plan.getByTestId('plan-step').count()
    check('plan: every step listed (1 vanilla + 4 Forge)', n === 5, String(n))
    const argv = (await plan.getByTestId('plan-argv').first().innerText()).split('\n').map((s) => s.trim())
    check('plan: full installer argv', ['{runtime.java}', '-jar', '{artifact.installer}', '--installClient', '{layerFiles}'].every((a) => argv.includes(a)), argv.join(' '))
    await plan.scrollIntoViewIfNeeded()
    await shot('plan')
})

// ------------------------------------------------------------ Consent, then progress
await step('consent', async () => {
    await pick('Environment', /^Server/)
    await button(/^Deploy$/).click()
    const dialog = page.getByTestId('install-consent')
    await dialog.waitFor({ timeout: 5000 })
    check('consent: exec statement', await dialog.getByTestId('consent-exec').isVisible())
    const trust = await dialog.getByTestId('consent-trust').innerText()
    check('consent: trust line names the publisher and plan', /Published by TMC/.test(trust) && /c0ffee/.test(trust), trust)
    check('consent: EULA checkbox (server)', await dialog.getByTestId('consent-eula').isVisible())
    const install = button(/^Install$/, dialog)
    check('consent: Install disabled until the EULA is ticked', await install.isDisabled())
    await shot('consent')
    await dialog.getByTestId('consent-eula').locator('input').check()
    check('consent: Install enabled once ticked', await install.isEnabled())
    await install.click()

    const progress = page.getByTestId('deploy-progress')
    await progress.waitFor({ timeout: 5000 })
    const cons = await calls('install_plan_consent')
    check('consent: recorded against the plan hash', cons.length === 1 && /^c0ffee/.test(String(cons[0].args.planHash)), JSON.stringify(cons.map((c) => c.args)))
    await progress.getByText('Installed and deployed.').waitFor({ timeout: 8000 })
    const statuses = await progress.getByTestId('progress-step').evaluateAll((els) => els.map((e) => e.getAttribute('data-status')))
    check('progress: every step done', statuses.length === 3 && statuses.every((s) => s === 'done'), statuses.join(','))
    check('progress: sandbox_deploy ran once', (await calls('sandbox_deploy')).length === 1)
    await shot('progress', progress)
    await button(/^Close$/, progress).click()
    await progress.waitFor({ state: 'detached', timeout: 5000 })
})

// ------------------------------------------------------------ A failed step
await step('failure', async () => {
    await page.evaluate(() => (globalThis.__reviewDeployFail = true))
    await button(/^Deploy$/).click()
    const progress = page.getByTestId('deploy-progress')
    await progress.waitFor({ timeout: 5000 })
    await progress.getByText(/Install failed\./).waitFor({ timeout: 8000 })
    const failedStep = progress.locator('[data-testid=progress-step][data-status=failed]')
    check('failure: the installer step is marked failed', (await failedStep.count()) === 1 && /Run/i.test(await failedStep.innerText()), await failedStep.first().innerText().catch(() => ''))
    const log = progress.getByTestId('progress-log')
    await log.locator('summary').click()
    const text = await log.locator('pre').innerText()
    check('failure: the log opens with the installer output', /503/.test(text), text.slice(0, 120))
    await shot('failed', progress)
    await button(/^Close$/, progress).click()
    await page.evaluate(() => (globalThis.__reviewDeployFail = false))
})

// ------------------------------------------------------------ Custom version
await step('custom', async () => {
    await combo('Game version').click()
    await page.getByRole('option', { name: /Custom…/ }).first().click()
    const input = page.getByRole('textbox', { name: 'Game version' })
    await input.fill('../x')
    check('custom: ../x refused', (await input.getAttribute('aria-invalid')) === 'true')
    await shot('custom-refused')
    await input.press('Enter')
    check('custom: ../x never patched', !(await calls('sandbox_patch')).some((c) => c.args.patch?.gameVersion === '../x'))
    await input.fill('1.21.4-custom')
    check('custom: 1.21.4-custom accepted', (await input.getAttribute('aria-invalid')) !== 'true')
    await input.press('Enter')
    await page.waitForTimeout(400)
    check('custom: 1.21.4-custom patched', (await calls('sandbox_patch')).some((c) => c.args.patch?.gameVersion === '1.21.4-custom'))
    check('custom: shown as the current version', (await combo('Game version').innerText()).includes('1.21.4-custom'))
})

// ------------------------------------------------------------ Offline
await step('offline', async () => {
    await page.evaluate(() => (globalThis.__reviewCatalogMode = 'stale'))
    await revisit()
    const banner = page.getByTestId('catalog-offline')
    await banner.waitFor({ timeout: 8000 })
    const text = await banner.innerText()
    check('offline: stale list banner with its date', /Offline/.test(text) && /2026/.test(text), text)
    check('offline: pickers still work from the cached list', await combo('Game version').isVisible())
    await banner.scrollIntoViewIfNeeded()
    await shot('offline')
})

check('no page errors', errors.length === 0, errors.join(' | '))
await browser.close()
console.log(failed ? `${failed} FAILED` : 'ALL PASS')
process.exit(failed ? 1 : 0)
