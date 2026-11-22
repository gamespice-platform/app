/**
 * The logic behind the game version, loader and loader version pickers and
 * the install plan panel, kept apart from React so vitest can hold it.
 *
 * Nothing here is game-specific: every list comes from the catalog of the
 * sandbox's own game (`catalog_get`), and a game with no catalog rows gets
 * the free-text fields instead.
 */
import type { SelectOption } from '~/components/select'
import type {
    GameLoaderWireT,
    GameVersionWireT,
    LoaderVersionWireT,
} from '~/lib/api/contract'
import type { InstallPlanT, PlannedLayerT } from '~/lib/ipc/schemas'
import { ct } from './text'

/** `docs/loaders-and-versions.md` "Custom versions"; Rust checks it again. */
export const CUSTOM_VERSION = /^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$/

/** Why `text` cannot be a custom version, or `null` when it can. */
export function checkCustomVersion(text: string): string | null {
    const value = text.trim()
    if (!CUSTOM_VERSION.test(value) || value.includes('..')) {
        return ct('catalog.version.customBad')
    }
    return null
}

type Deprecation = GameVersionWireT['deprecated']

function deprecationHint(d: Deprecation): string | undefined {
    if (!d) return undefined
    const word = d.withdrawn
        ? ct('catalog.version.withdrawn')
        : ct('catalog.version.deprecated')
    return d.note ? `${word}: ${d.note}` : word
}

function joinHints(...parts: (string | undefined | false)[]): string | undefined {
    const kept = parts.filter((p): p is string => Boolean(p))
    return kept.length ? kept.join(' · ') : undefined
}

/**
 * The game version rows: "Any version" first, then the catalog's (newest
 * first, as served), then the current value when the catalog does not list
 * it (a custom version, or one past the last-N window).
 */
export function versionOptions(
    items: GameVersionWireT[],
    current: string | null
): SelectOption<string>[] {
    const rows: SelectOption<string>[] = [
        { value: '', label: ct('catalog.version.any') },
    ]
    for (const v of items) {
        if (v.deprecated?.withdrawn && v.versionId !== current) continue
        rows.push({
            value: v.versionId,
            label: v.label ?? v.versionId,
            hint: joinHints(
                v.channel !== 'STABLE' && (v.channelLabel ?? v.channel.toLowerCase()),
                deprecationHint(v.deprecated),
                !v.managed && ct('catalog.version.unmanaged')
            ),
        })
    }
    if (current && !items.some((v) => v.versionId === current)) {
        rows.push({
            value: current,
            label: ct('catalog.version.unknown', { value: current }),
            hint: ct('catalog.version.customHint'),
        })
    }
    return rows
}

function sideHint(sides: GameLoaderWireT['sides']): string | undefined {
    if (sides === 'SERVER') return ct('catalog.loader.serverOnly')
    if (sides === 'CLIENT') return ct('catalog.loader.clientOnly')
    return undefined
}

/**
 * The loader rows: "None (vanilla)" first, then the catalog's by `position`,
 * then the current value when the catalog does not know it. Free text from
 * before the catalog (`optifine`) stays as it is and shows as unknown.
 */
export function loaderOptions(
    loaders: GameLoaderWireT[],
    current: string | null
): SelectOption<string>[] {
    const rows: SelectOption<string>[] = [
        { value: '', label: ct('catalog.loader.none') },
    ]
    const sorted = [...loaders].sort((a, b) => a.position - b.position)
    for (const l of sorted) {
        rows.push({
            value: l.key,
            label: l.name,
            hint: joinHints(
                sideHint(l.sides),
                !l.managed && ct('catalog.version.unmanaged')
            ),
        })
    }
    if (current && !findLoader(loaders, current)) {
        rows.push({
            value: current,
            label: ct('catalog.loader.unknown', { value: current }),
            hint: ct('catalog.loader.unknownHint'),
        })
    }
    return rows
}

/** The catalog loader `value` names; loader keys compare case-insensitively. */
export function findLoader(
    loaders: GameLoaderWireT[],
    value: string | null
): GameLoaderWireT | undefined {
    if (!value) return undefined
    const key = value.toLowerCase()
    return loaders.find((l) => l.key.toLowerCase() === key)
}

/** The loader version rows, as served (newest first), plus a custom current. */
export function loaderVersionOptions(
    items: LoaderVersionWireT[],
    current: string | null
): SelectOption<string>[] {
    const rows: SelectOption<string>[] = items
        .filter((v) => !v.deprecated?.withdrawn || v.version === current)
        .map((v) => ({
            value: v.version,
            label: v.version,
            hint: joinHints(
                v.recommended && ct('catalog.loaderVersion.recommended'),
                v.channel !== 'STABLE' && v.channel.toLowerCase(),
                deprecationHint(v.deprecated)
            ),
        }))
    if (current && !items.some((v) => v.version === current)) {
        rows.push({
            value: current,
            label: ct('catalog.version.unknown', { value: current }),
            hint: ct('catalog.version.customHint'),
        })
    }
    return rows
}

/** The build a fresh pick lands on: the recommended one, else the newest. */
export function defaultLoaderVersion(
    items: LoaderVersionWireT[]
): string | null {
    const live = items.filter((v) => !v.deprecated?.withdrawn)
    const pick = live.find((v) => v.recommended) ?? live[0]
    return pick?.version ?? null
}

export type LoaderVersionCarry = {
    /** The loader version to store. */
    value: string | null
    /** The one-line notice under the field, when the old one had to go. */
    notice: string | null
}

/**
 * After the game version changed: keep `current` when the new version has a
 * build of it, else move to the recommended or newest build and say so.
 */
export function carryLoaderVersion(
    items: LoaderVersionWireT[],
    current: string | null,
    names: { loader: string; gameVersion: string }
): LoaderVersionCarry {
    if (current && items.some((v) => v.version === current)) {
        return { value: current, notice: null }
    }
    const next = defaultLoaderVersion(items)
    if (!current) return { value: next, notice: null }
    if (!next) {
        return {
            value: current,
            notice: ct('catalog.loaderVersion.none', names),
        }
    }
    return {
        value: next,
        notice: ct('catalog.loaderVersion.moved', {
            ...names,
            from: current,
            to: next,
        }),
    }
}

export type Provenance = {
    source: 'remote' | 'cache' | 'compiled'
    stale: boolean
    fetchedAt: number | null
}

/** The offline line over a picker, or `null` for a fresh list. */
export function provenanceNotice(
    p: Provenance,
    formatDate: (ms: number) => string = (ms) => new Date(ms).toLocaleString()
): string | null {
    if (p.source === 'compiled') return ct('catalog.source.compiled')
    if (p.stale) {
        return ct('catalog.source.stale', {
            date: p.fetchedAt == null ? '?' : formatDate(p.fetchedAt),
        })
    }
    return null
}

/** A catalog with nothing in it: the game keeps its free-text fields. */
export function hasCatalog(view: {
    loaders: unknown[]
    versions: { items: unknown[] }
}): boolean {
    return view.loaders.length > 0 || view.versions.items.length > 0
}

// ---------------------------------------------------------------------------
// The install plan, as the user reads it.

/** A piece of step text: plain, or a placeholder only the install fills in. */
export type Piece = { text: string; token: boolean }

/**
 * `template` with each `{name}` the layer knows replaced by its value. The
 * ones it cannot know before running (`{artifact.installer}`,
 * `{runtime.java}`, `{layerFiles}`) stay, marked as tokens.
 */
export function expand(template: string, values: Record<string, string>): Piece[] {
    const out: Piece[] = []
    let rest = template
    const re = /\{([A-Za-z0-9_.]+)\}/
    for (let guard = 0; guard < 256; guard++) {
        const m = re.exec(rest)
        if (!m) break
        const name = m[1] ?? ''
        const known = values[name.startsWith('var.') ? name.slice(4) : name]
        if (m.index > 0) out.push({ text: rest.slice(0, m.index), token: false })
        out.push(
            known === undefined
                ? { text: m[0], token: true }
                : { text: known, token: false }
        )
        rest = rest.slice(m.index + m[0].length)
    }
    if (rest) out.push({ text: rest, token: false })
    // Join neighbouring plain text so a fully known value reads as one string.
    return out.reduce<Piece[]>((acc, p) => {
        const last = acc[acc.length - 1]
        if (last && !last.token && !p.token) last.text += p.text
        else acc.push({ ...p })
        return acc
    }, [])
}

export function piecesText(pieces: Piece[]): string {
    return pieces.map((p) => p.text).join('')
}

function str(v: unknown): string | null {
    return typeof v === 'string' ? v : null
}

/** A step's `{root, path}` as `root/path`, expanded. */
function pathOf(v: unknown, values: Record<string, string>): Piece[] | null {
    if (typeof v === 'string') return expand(v, values)
    if (v && typeof v === 'object') {
        const o = v as Record<string, unknown>
        const path = str(o.path)
        if (path === null) return null
        const root = str(o.root)
        return expand(root ? `${root}/${path}` : path, values)
    }
    return null
}

export type StepView = {
    action: string
    /** What the step works on: a file, a download target. */
    target: Piece[] | null
    /** `runInstaller`: the program, then each argument, one element each. */
    argv: Piece[][] | null
    cwd: Piece[] | null
    /** `download`: the artifact name it fetches. */
    artifact: string | null
    raw: Record<string, unknown>
}

/** One plan step, laid out for the panel. Unknown actions show their JSON. */
export function stepView(
    raw: Record<string, unknown>,
    values: Record<string, string>
): StepView {
    const action = str(raw.action) ?? '?'
    const view: StepView = {
        action,
        target: null,
        argv: null,
        cwd: null,
        artifact: str(raw.artifact),
        raw,
    }
    if (action === 'runInstaller') {
        const program = str(raw.program) ?? '?'
        const args = Array.isArray(raw.args) ? raw.args : []
        view.argv = [program, ...args.map((a) => String(a))].map((a) =>
            expand(a, values)
        )
        view.cwd = str(raw.cwd) === null ? null : expand(String(raw.cwd), values)
    } else {
        view.target =
            pathOf(raw.to, values) ?? pathOf(raw.path, values) ?? pathOf(raw.from, values)
    }
    return view
}

/** Every step of the plan in run order, numbered across layers. */
export function planSteps(
    plan: Pick<InstallPlanT, 'layers'>
): { layer: PlannedLayerT; steps: { n: number; view: StepView }[] }[] {
    let n = 0
    return plan.layers.map((layer) => ({
        layer,
        steps: layer.steps.map((raw) => ({ n: ++n, view: stepView(raw, layer.values) })),
    }))
}

/** The host the installer program comes from, for the exec statement. */
export function execHost(plan: Pick<InstallPlanT, 'layers'>): string {
    const hosts = plan.layers
        .filter((l) => l.exec)
        .flatMap((l) => l.downloadHosts)
    return [...new Set(hosts)].join(', ') || '?'
}

/** The trust line of the consent dialog. */
export function trustLine(plan: Pick<InstallPlanT, 'trust' | 'planHash'>): string {
    const hash = plan.planHash.slice(0, 12)
    if (plan.trust === 'official') return ct('catalog.consent.trust.official', { hash })
    if (plan.trust === 'compiled') return ct('catalog.consent.trust.compiled', { hash })
    return ct('catalog.consent.trust.local', { hash })
}

/** `exec:` and `runtime:` consents are what the dialog itself says; the rest
 * (a EULA) each get a checkbox. */
export function checkboxConsents<T extends { key: string }>(consents: T[]): T[] {
    return consents.filter(
        (c) => !c.key.startsWith('exec:') && !c.key.startsWith('runtime:')
    )
}
