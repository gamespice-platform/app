import type { DetectedGameT } from '~/lib/ipc/schemas'

/**
 * The rows of Library -> My apps, as pure data.
 *
 * Three states, in the order a person cares about them:
 *   * `installed`: a folder is set for the game, or it has a sandbox here;
 *   * `detected`: the "Find my games" scan matched it and nothing is applied;
 *   * `other`: a catalogue game with nothing on this device. Still listed,
 *     because "Manage" is also how a folder is set by hand.
 */
export type CatalogGame = {
    id: number
    name: string
    icon: string | null
    /** The catalogue's `url`; lower-cased it is the plugin slug. */
    url: string | null
}

export type ManageStatus = 'installed' | 'detected' | 'other'

export type ManageRow = {
    appId: number
    name: string
    icon: string | null
    slug: string | null
    dir: string | null
    hasSandbox: boolean
    status: ManageStatus
    /** Scan results that matched this game. */
    detected: DetectedGameT[]
}

/** The slug a plugin folder is named by: lower-cased, as the app loads them. */
export function slugOf(game: Pick<CatalogGame, 'url'>): string | null {
    const slug = game.url?.trim().toLowerCase()

    return slug ? slug : null
}

const STATUS_ORDER: Record<ManageStatus, number> = {
    installed: 0,
    detected: 1,
    other: 2,
}

/**
 * Which game id a scan result belongs to: the id Rust resolved, else the
 * catalogue game whose slug it names. Null when neither places it.
 */
function placeOf(d: DetectedGameT, bySlug: Map<string, number>): number | null {
    if (d.appId !== null && d.appId !== undefined) return d.appId

    const slug = d.slug?.toLowerCase()

    return slug ? (bySlug.get(slug) ?? null) : null
}

function slugIndex(catalogue: CatalogGame[]): Map<string, number> {
    const out = new Map<string, number>()

    for (const g of catalogue) {
        const slug = slugOf(g)

        if (slug && !out.has(slug)) out.set(slug, g.id)
    }

    return out
}

/** Is `a` the same folder as `b`, ignoring slash direction and a trailing one? */
export function sameDir(a: string | null, b: string | null): boolean {
    if (a === null || b === null) return false

    const norm = (p: string) => {
        const t = p.replace(/\\/g, '/').replace(/\/+$/, '')

        return /^[a-z]:/i.test(t) ? t.toLowerCase() : t
    }

    return norm(a) === norm(b)
}

export function buildManageRows(input: {
    catalogue: CatalogGame[]
    gameDirs: Record<string, string>
    sandboxAppIds: number[]
    detected: DetectedGameT[]
    search?: string
}): ManageRow[] {
    const withSandbox = new Set(input.sandboxAppIds)
    const known = new Map(input.catalogue.map((g) => [g.id, g]))

    // A folder or sandbox for a game the catalogue no longer lists is still a
    // game on this machine; it must stay reachable to be cleared.
    const ids = new Set<number>(known.keys())

    for (const key of Object.keys(input.gameDirs)) {
        const id = Number(key)

        if (Number.isFinite(id)) ids.add(id)
    }

    for (const id of withSandbox) ids.add(id)

    const bySlug = slugIndex(input.catalogue)
    const placed = new Map<number, DetectedGameT[]>()
    const detectedNames = new Map<number, string>()

    for (const d of input.detected) {
        const id = placeOf(d, bySlug)

        if (id === null) continue

        placed.set(id, [...(placed.get(id) ?? []), d])
        if (!detectedNames.has(id)) detectedNames.set(id, d.name)
        // A result for a game the catalogue facet omits is still a game.
        ids.add(id)
    }

    const term = (input.search ?? '').trim().toLowerCase()

    const rows = [...ids].map((appId): ManageRow => {
        const game = known.get(appId)
        const slug = game ? slugOf(game) : null
        const dir = input.gameDirs[String(appId)] ?? null
        const hasSandbox = withSandbox.has(appId)
        const detected = placed.get(appId) ?? []
        const installed = dir !== null || hasSandbox

        return {
            appId,
            name:
                game?.name.trim() || detectedNames.get(appId) || `Game #${appId}`,
            icon: game?.icon ?? null,
            slug,
            dir,
            hasSandbox,
            detected,
            status: installed
                ? 'installed'
                : detected.length > 0
                  ? 'detected'
                  : 'other',
        }
    })

    return rows
        .filter((row) => !term || row.name.toLowerCase().includes(term))
        .sort(
            (a, b) =>
                STATUS_ORDER[a.status] - STATUS_ORDER[b.status] ||
                a.name.localeCompare(b.name)
        )
}

/**
 * Slugged results nothing can place: the app has a rule, the site has no game
 * for it. They get their own group so "Use this" can say why it fails.
 */
export function unplacedDetected(input: {
    catalogue: CatalogGame[]
    detected: DetectedGameT[]
}): DetectedGameT[] {
    const bySlug = slugIndex(input.catalogue)

    return input.detected.filter((d) => d.slug && placeOf(d, bySlug) === null)
}

/** Results the app has no rule for at all (no slug): shown as a count and a list. */
export function unsupportedDetected(detected: DetectedGameT[]): DetectedGameT[] {
    return detected.filter((d) => !d.slug)
}

/** Kept for callers that only want the number. */
export function unmatchedCount(detected: DetectedGameT[]): number {
    return unsupportedDetected(detected).length
}
