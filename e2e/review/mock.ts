/**
 * Review harness bridge: stands in for Rust behind `@tauri-apps/api` so the
 * real web UI runs in a plain browser. API and avatar commands go to the dev
 * backend through the harness proxy (vite.config.ts adds the dev token);
 * filesystem commands are faked with a small in-memory store; anything else
 * answers the smallest value its zod schema accepts.
 */
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'

import { fixture } from './fixtures'

type Def = { type: string; [k: string]: unknown }
type Schema = { _zod?: { def: Def } }

const g = globalThis as Record<string, unknown>

/** Smallest value a zod v4 schema accepts. */
export function fake(schema: unknown, depth = 0): unknown {
    const def = (schema as Schema)?._zod?.def

    if (!def || depth > 12) return null

    switch (def.type) {
        case 'object': {
            const out: Record<string, unknown> = {}
            const shape = def.shape as Record<string, unknown>

            for (const [k, v] of Object.entries(shape)) {
                const t = (v as Schema)._zod?.def.type

                if (t === 'optional') continue
                out[k] = fake(v, depth + 1)
            }

            return out
        }
        case 'array': return []
        case 'record': case 'map': return {}
        case 'set': return []
        case 'string': return ''
        case 'number': case 'int': return 0
        case 'bigint': return 0
        case 'boolean': return false
        case 'null': return null
        case 'void': case 'undefined': return undefined // z.void() refuses null
        case 'nullable': return null
        case 'optional': return undefined
        case 'default': return typeof def.defaultValue === 'function' ? (def.defaultValue as () => unknown)() : def.defaultValue
        case 'catch': case 'readonly': case 'nonoptional': case 'prefault': return fake(def.innerType, depth + 1)
        case 'enum': return Object.values(def.entries as object)[0]
        case 'literal': return (def.values as unknown[])[0]
        case 'union': return fake((def.options as unknown[])[0], depth + 1)
        case 'intersection': return { ...(fake(def.left, depth + 1) as object), ...(fake(def.right, depth + 1) as object) }
        case 'tuple': return (def.items as unknown[]).map((i) => fake(i, depth + 1))
        case 'pipe': return fake(def.in, depth + 1)
        case 'lazy': return fake((def.getter as () => unknown)(), depth + 1)
        case 'custom': return new ArrayBuffer(0)
        default: return null
    }
}

async function json(res: Response): Promise<unknown> {
    const text = await res.text()
    let body: { ok?: boolean; data?: unknown; message?: string; error?: { code?: string; message?: string } | string } | null = null

    try { body = JSON.parse(text) } catch { /* not JSON */ }

    if (res.status === 404) return null
    if (res.status === 401) throw { code: 'not_authenticated', message: 'Sign in again.' }
    if (body && typeof body.ok === 'boolean') {
        if (body.ok) return body.data
        const e = body.error
        // `{ ok: false, code, message }` as the bridge's `unwrap` reads it; older routes nest an `error`.
        throw { code: 'api', message: body.message ?? (typeof e === 'string' ? e : (e?.message ?? `HTTP ${res.status}`)) }
    }
    if (!res.ok) throw { code: 'api', message: `HTTP ${res.status}` }

    return body
}

const log: { cmd: string; args: unknown }[] = []
g.__reviewLog = log

/** Filesystem state the faked commands share. */
const sandboxes = new Map<number, Record<string, unknown>>()
let nextSandbox = 1
let settings: Record<string, unknown> | null = null
/** Installed TMC games by app id; the published build every install gets. */
const games = new Map<number, Record<string, unknown>>()
const REVIEW_BUILD = '1.20261113.1637'
g.__reviewGames = games
g.__reviewSandboxes = sandboxes

// ------------------------------------------------------------ Game versions and loaders
// `minecraft` has a catalog; every other game has none (free-text fields).
// A script sets `__reviewCatalogMode = 'stale'` for the offline banner and
// `__reviewDeployFail = true` for an installer that exits non-zero.
const ISO = '2026-11-01T00:00:00Z'
const VERSIONS = ['1.21.4', '1.21.3', '1.21.1', '1.20.6', '1.20.1', '1.19.2', '1.18.2']
const catalogVersions = () => [
    { versionId: '25w14a', label: null, channel: 'PREVIEW', channelLabel: 'Snapshot', sortKey: 900, releasedAt: ISO, deprecated: null, managed: true },
    ...VERSIONS.map((v, i) => ({ versionId: v, label: null, channel: 'STABLE', channelLabel: null, sortKey: 800 - i, releasedAt: ISO, deprecated: null, managed: true })),
]
const LOADERS = [
    { key: 'forge', name: 'Forge', description: 'The oldest loader; most big mods.', sides: 'BOTH', onlineNote: null, position: 1, customVersions: true, managed: true },
    { key: 'fabric', name: 'Fabric', description: 'Light and quick to update.', sides: 'BOTH', onlineNote: null, position: 2, customVersions: true, managed: true },
    { key: 'neoforge', name: 'NeoForge', description: 'The Forge fork most 1.20.2+ mods target.', sides: 'BOTH', onlineNote: null, position: 3, customVersions: true, managed: true },
    { key: 'quilt', name: 'Quilt', description: 'Fabric-compatible.', sides: 'BOTH', onlineNote: null, position: 4, customVersions: true, managed: true },
]
const BUILDS: Record<string, Record<string, string[]>> = {
    forge: { '1.21.4': ['54.1.0', '54.0.26', '54.0.12'], '1.21.1': ['52.0.40', '52.0.31'], '1.20.1': ['47.3.0', '47.2.0'] },
    fabric: { '*': ['0.16.9', '0.16.7', '0.15.11'] },
    neoforge: { '1.21.4': ['21.4.75', '21.4.50'], '1.21.1': ['21.1.90'] },
    quilt: { '*': ['0.27.1', '0.26.4'] },
}
const provenance = () => g.__reviewCatalogMode === 'stale'
    ? { source: 'cache', stale: true, fetchedAt: Date.parse('2026-11-18T09:30:00Z') }
    : { source: 'remote', stale: false, fetchedAt: Date.now() }
function loaderBuilds(loader: string, gameVersion: string | null) {
    const table = BUILDS[loader] ?? {}
    const list = table['*'] ?? (gameVersion ? table[gameVersion] : Object.values(table).flat()) ?? []
    return list.map((version, i) => ({ version, channel: 'STABLE', sortKey: 100 - i, recommended: i === 0, custom: false, deprecated: null }))
}

const listeners = new Map<string, number[]>()
function emit(event: string, payload: unknown) {
    const internals = g.__TAURI_INTERNALS__ as { runCallback(id: number, data: unknown): void }
    for (const id of listeners.get(event) ?? []) internals.runCallback(id, { event, id, payload })
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

/** The install plan `install_plan_preview` would resolve for a sandbox, from the real Forge manifest's steps. */
function planFor(s: Record<string, unknown>) {
    const gameVersion = s.gameVersion as string | null
    const loader = s.loader as string | null
    const loaderVersion = (s.loaderVersion as string | null) ?? null
    if (s.appSlug !== 'minecraft' || !gameVersion) return null
    const server = s.environment === 'server'
    const side = server ? 'server' : 'client'
    const layers: Record<string, unknown>[] = [{
        manifestKey: `minecraft/version@${gameVersion}`, revision: 1, title: `Minecraft ${gameVersion}`, layerHash: 'a1'.repeat(32), exec: false,
        steps: [{ action: 'download', artifact: server ? 'server' : 'client', to: { root: 'layer', path: server ? 'files/server.jar' : `files/versions/${gameVersion}/${gameVersion}.jar` } }],
        values: { gameVersion }, downloadHosts: ['piston-data.mojang.com'],
    }]
    if (loader === 'forge' && loaderVersion) {
        const installer = { action: 'runInstaller', program: '{runtime.java}', args: ['-jar', '{artifact.installer}', server ? '--installServer' : '--installClient', '{layerFiles}'], cwd: '{layerWork}', network: true, timeoutSecs: server ? 1800 : 900 }
        layers.push({
            manifestKey: `minecraft/loader/forge@${gameVersion}-${loaderVersion}`, revision: 1, title: `Forge ${loaderVersion}`, layerHash: 'b2'.repeat(32), exec: true,
            steps: server
                ? [{ action: 'download', artifact: 'installer', to: { root: 'downloads', path: 'forge-{gameVersion}-{loaderVersion}-installer.jar' } }, installer]
                : [
                    { action: 'download', artifact: 'installer', to: { root: 'downloads', path: 'forge-{gameVersion}-{loaderVersion}-installer.jar' } },
                    { action: 'writeText', ifMissing: true, content: '{"profiles":{}}', path: { root: 'layer', path: 'files/launcher_profiles.json' } },
                    installer,
                    { action: 'remove', path: { root: 'layer', path: 'files/launcher_profiles.json' } },
                ],
            values: { gameVersion, loaderVersion, launchVersionId: `${gameVersion}-forge-${loaderVersion}` },
            downloadHosts: ['maven.minecraftforge.net'],
        })
    }
    const exec = layers.some((l) => l.exec)
    const consents = [
        ...(exec ? [{ key: 'exec:minecraft/loader/forge', text: 'Runs the Forge installer with Java.', url: null }] : []),
        ...(server ? [{ key: 'eula', text: 'I accept the Minecraft End User License Agreement.', url: 'https://aka.ms/MinecraftEULA' }] : []),
    ]
    return {
        planHash: `c0ffee${gameVersion}${loader ?? ''}${loaderVersion ?? ''}${side}`.replace(/[^0-9a-z]/g, '').padEnd(64, '0'),
        appSlug: 'minecraft', trust: 'official', side, platform: 'linux-x64', layers,
        runtimes: exec ? [{ kind: 'java', major: 21, for: 'install', offer: null }] : [],
        consents, exec,
    }
}
const consented = new Set<string>()

mockWindows('main')
g.__TAURI_OS_PLUGIN_INTERNALS__ = { platform: 'linux', os_type: 'linux', family: 'unix', version: '6', arch: 'x86_64', exe_extension: '', eol: '\n', hostname: 'review' }

mockIPC(async (cmd, args) => {
    const schema = g.__reviewSchema
    const a = (args ?? {}) as Record<string, unknown>

    log.push({ cmd, args: a })

    switch (cmd) {
        case 'api_get': {
            const q = new URLSearchParams((a.query as [string, string][] | undefined) ?? [])
            const fx = g.__reviewFixtures ? fixture('GET', String(a.path), q, undefined) : undefined
            if (fx !== undefined) return fx
            const qs = q.toString()
            const body = await json(await fetch(`/__api${a.path}${qs ? `?${qs}` : ''}`))
            // No dev server publishes a connect link; a flow script that drives
            // Join -> play_connect lends one to every server it opens.
            const srv = (body as { summary?: { server?: Record<string, unknown> } } | null)?.summary?.server
            if (g.__reviewConnectUrl && srv && String(a.path).startsWith('/content/server/')) srv.connectUrl = `steam://connect/${srv.host}:${srv.port}`
            return body
        }
        case 'api_send': {
            const fx = g.__reviewFixtures ? fixture(String(a.method), String(a.path), new URLSearchParams(), a.body as Record<string, unknown>) : undefined
            if (fx !== undefined) return fx
            return json(await fetch(`/__api${a.path}`, {
                method: a.method as string,
                headers: { 'content-type': 'application/json' },
                body: a.body === undefined ? undefined : JSON.stringify(a.body),
            }))
        }
        case 'avatar_manifest': return json(await fetch('/__avatar/manifest'))
        case 'avatar_library': return json(await fetch(`/__avatar/library?app=${a.appId}`))
        case 'avatar_me': return json(await fetch(`/__avatar/me?app=${a.appId}`))
        case 'avatar_me_put':
            return json(await fetch('/__avatar/me', { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify(a.body) }))
        case 'avatar_asset': {
            const u = new URL(String(a.url))
            const res = await fetch(`/__asset${u.pathname}${u.search}`)
            if (!res.ok) throw { code: 'api', message: `HTTP ${res.status}` }
            return res.arrayBuffer()
        }
        case 'session_state': {
            const me = (g.__reviewFixtures ? fixture('GET', '/me', new URLSearchParams(), undefined) : await json(await fetch('/__api/me')).catch(() => null)) as Record<string, unknown> | null
            const userSchema = (schema as { shape: { user: { unwrap(): unknown } } }).shape.user.unwrap()
            const user = me ? (fake(userSchema) as Record<string, unknown>) : null
            if (user && me) for (const k of Object.keys(user)) if (k in me) user[k] = (me as Record<string, unknown>)[k]
            if (user && me && 'user' in me) Object.assign(user, me.user)
            if (user) user.id = String(user.id)
            return { signedIn: !!me, pending: false, user }
        }
        case 'api_env': return { base: 'https://tmcdev.net', isProd: false, version: '0.0.0-review' }
        case 'settings_get': return (settings ??= { ...(fake(schema) as Record<string, unknown>), uiScale: 1, theme: 'system', liveLatency: true, latencyIntervalMs: 1000, latencyConcurrency: 8, downloadConcurrency: 3, autoUpdateCheck: true }) // Rust's defaults (core/src/settings.rs) where fake()'s zeros read as nonsense ("every 0 seconds") // a zero scale makes 1rem = 0px; the page's colorScheme picks light or dark
        case 'settings_patch': return (settings = { ...(settings ?? {}), ...(a.patch as object ?? a) })
        // Fixture mode supports the dev site's one modded game, so the new
        // sandbox form has something to offer.
        case 'library_supported_apps': return g.__reviewFixtures ? ['css'] : []
        case 'sandbox_list':
            return [...sandboxes.values()].filter((s) => a.appId == null || s.appId === a.appId)
        case 'sandbox_get': return sandboxes.get(Number(a.id)) ?? null
        case 'sandbox_create': {
            const row = fake((schema as { _zod: { def: Def } })) as Record<string, unknown>
            const input = (a.new ?? a) as Record<string, unknown>
            const id = nextSandbox++
            Object.assign(row, { id, appId: input.appId, name: input.name, targetDir: g.__reviewFixtures ? '/review/game' : null, appSlug: input.appSlug ?? null, appName: input.appName ?? null, isDefault: sandboxes.size === 0 })
            sandboxes.set(id, row)
            return row
        }
        case 'sandbox_set_default': {
            for (const s of sandboxes.values()) if (s.appId === sandboxes.get(Number(a.id))?.appId) s.isDefault = s.id === Number(a.id)
            return fake(schema)
        }
        case 'sandbox_delete': sandboxes.delete(Number(a.id)); return fake(schema)
        case 'sandbox_patch': {
            const s = sandboxes.get(Number(a.id))
            if (s) Object.assign(s, a.patch ?? {})
            return s ?? fake(schema)
        }
        // ---------------------------------------------------- TMC games (fixture mode)
        // One target build per app at REVIEW_BUILD; a flow script ages an
        // install through `__reviewGames` to make an update appear.
        case 'games_platform': return g.__reviewFixtures ? { platform: 'LINUX_X64', installable: true } : fake(schema)
        case 'games_list': return [...games.values()]
        case 'game_install': {
            const r = (a.request ?? a) as { appId: number; name: string; slug?: string }
            const now = Date.now()
            const prev = games.get(r.appId)
            const row = { appId: r.appId, slug: r.slug ?? null, name: r.name, platform: 'LINUX_X64', version: REVIEW_BUILD, dir: `/review/games/${r.slug ?? r.appId}`, entry: null, args: [], sizeBytes: 52_428_800, installedMs: prev?.installedMs ?? now, updatedMs: now, autoUpdate: prev?.autoUpdate ?? false }
            games.set(r.appId, row)
            return row
        }
        case 'game_uninstall': games.delete(Number(a.appId)); return undefined
        case 'games_check_updates':
            return [...games.values()].map((r) => ({ ...r, available: REVIEW_BUILD, updateAvailable: r.version !== REVIEW_BUILD, notes: null }))
        case 'game_set_auto_update': {
            const r = games.get(Number(a.appId))
            if (r) r.autoUpdate = !!a.enabled
            return fake(schema)
        }
        // ---------------------------------------------------- Mods in a sandbox (fixture mode)
        case 'sandbox_install_item': {
            const s = sandboxes.get(Number(a.id))
            if (!s) throw { code: 'not_found', message: 'No such sandbox' }
            const mods = s.mods as Record<string, unknown>[]
            const modKey = `${a.kind}:${a.itemId}`
            const already = mods.some((m) => m.modKey === modKey)
            if (!already) mods.push({ modKey, kind: a.kind, itemId: Number(a.itemId), name: `Item ${a.itemId}`, enabled: true, priority: mods.length, releaseId: 1, version: '1.0', stagedAt: new Date().toISOString(), lastError: null })
            s.deployedFiles = Number(s.deployedFiles ?? 0) + 3
            s.needsDeploy = false
            const deployed = { ...(fake((schema as { shape: { deployed: { unwrap(): unknown } } }).shape.deployed.unwrap()) as object), placed: 3, backedUp: 1 }
            return { subscribed: true, alreadyPresent: already, staged: { modKey, ok: true, rule: 'files', files: 3, error: null }, deployed: a.deploy === false ? null : deployed, warnings: [] }
        }
        case 'sandbox_updates': {
            const out: Record<string, unknown>[] = []
            for (const s of sandboxes.values())
                for (const m of s.mods as Record<string, unknown>[])
                    if (m.version !== '2.0') out.push({ sandboxId: s.id, sandboxName: s.name, modKey: m.modKey, name: m.name, fromVersion: m.version, toVersion: '2.0', releaseId: 2 })
            return g.__reviewFixtures ? out : []
        }
        case 'sandbox_stage': {
            const s = sandboxes.get(Number(a.id))
            const mods = (s?.mods ?? []) as Record<string, unknown>[]
            if (a.force) for (const m of mods) Object.assign(m, { version: '2.0', releaseId: 2 })
            return mods.map((m) => ({ modKey: m.modKey, ok: true, rule: 'files', files: 3, error: null }))
        }
        case 'sandbox_purge': {
            const s = sandboxes.get(Number(a.id))
            const removed = Number(s?.deployedFiles ?? 0)
            if (s) Object.assign(s, { deployedFiles: 0, needsDeploy: (s.mods as unknown[]).length > 0 })
            return { removed, restored: removed ? 1 : 0, kept: [], missing: 0, errors: [] }
        }
        // ---------------------------------------------------- Live server queries (fixture mode)
        case 'query_servers': case 'query_server': {
            const reqs = (cmd === 'query_server' ? [a.request] : a.requests) as { id: string; host: string; port: number }[]
            const out = reqs.map((r) => ({
                id: r.id,
                key: `${r.host}:${r.port}`,
                result: g.__reviewFixtures ? { online: true, rttMs: 42, protocol: 'A2S', name: null, map: 'de_dust2', game: null, version: null, players: 7, maxPlayers: 24, bots: 0, password: false, secure: true, playerList: [], rules: {}, queriedPort: r.port } : undefined,
                error: g.__reviewFixtures ? undefined : 'review: no network',
            }))
            return cmd === 'query_server' ? out[0] : out
        }
        case 'play_connect': {
            const session = fake(schema) as Record<string, unknown>
            return Object.assign(session, { id: 1, kind: fake((schema as { shape: { kind: unknown } }).shape.kind), label: `server ${a.serverId}`, startedMs: Date.now() })
        }
        case 'catalog_get': {
            const slug = String(a.appSlug ?? '')
            const known = slug === 'minecraft'
            return { ...provenance(), app: { id: Number(a.appId), slug }, loaders: known ? LOADERS : [], versions: { items: known ? catalogVersions() : [], nextCursor: null }, generatedAt: ISO }
        }
        case 'catalog_loader_versions': {
            const gv = (a.gameVersion as string | null | undefined) ?? null
            return { ...provenance(), loader: String(a.loader), gameVersion: gv, items: loaderBuilds(String(a.loader), gv), nextCursor: null }
        }
        case 'install_plan_preview': {
            const s = sandboxes.get(Number(a.id))
            const plan = s ? planFor(s) : null
            return { plan, needsConsent: !!plan && plan.consents.length > 0 && !consented.has(plan.planHash) }
        }
        case 'install_plan_consent': consented.add(String(a.planHash)); return null
        case 'sandbox_deploy': {
            const s = sandboxes.get(Number(a.id))
            const plan = s ? planFor(s) : null
            if (plan && plan.consents.length > 0 && !consented.has(plan.planHash)) throw { code: 'needs_consent', message: 'This install needs your consent first.' }
            for (const layer of plan?.layers ?? []) {
                const steps = layer.steps as Record<string, unknown>[]
                for (let index = 0; index < steps.length; index++) {
                    const base = { sandboxId: Number(a.id), planHash: plan?.planHash, layer: layer.manifestKey, index, total: steps.length }
                    emit('deploy://plan-step', { ...base, status: 'running' })
                    await sleep(120)
                    if (g.__reviewDeployFail && steps[index].action === 'runInstaller') {
                        const logTail = 'Extracting main jar...\nDownloading libraries\nException in thread "main" java.io.IOException: Server returned HTTP response code: 503\nThere was an error during installation'
                        emit('deploy://plan-step', { ...base, status: 'failed', logTail })
                        throw { code: 'invalid', message: 'The Forge installer exited with code 1.' }
                    }
                    emit('deploy://plan-step', { ...base, status: 'done' })
                }
            }
            if (s) s.needsDeploy = false
            return { ...(fake(schema) as object), requested: 'hardlink', used: 'hardlink', placed: 12 }
        }
        case 'plugin:event|listen': {
            const list = listeners.get(String(a.event)) ?? []
            list.push(Number(a.handler))
            listeners.set(String(a.event), list)
            return Number(a.handler)
        }
        case 'plugin:event|unlisten': {
            const list = listeners.get(String(a.event)) ?? []
            listeners.set(String(a.event), list.filter((h) => h !== Number(a.eventId)))
            return null
        }
        case 'plugin:event|emit': return null
        default:
            return fake(schema)
    }
})
