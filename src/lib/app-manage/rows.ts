import type { DetectedGameT } from '~/lib/ipc/schemas'

/**
 * The rows of Library -> Apps, as pure data.
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

    const term = (input.search ?? '').trim().toLowerCase()

    const rows = [...ids].map((appId): ManageRow => {
        const game = known.get(appId)
        const slug = game ? slugOf(game) : null
        const dir = input.gameDirs[String(appId)] ?? null
        const hasSandbox = withSandbox.has(appId)
        const detected = slug
            ? input.detected.filter((d) => d.slug?.toLowerCase() === slug)
            : []
        const installed = dir !== null || hasSandbox

        return {
            appId,
            name: game?.name.trim() || `Game #${appId}`,
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

/** Detected games the catalogue cannot place: shown as a count, not rows. */
export function unmatchedCount(detected: DetectedGameT[]): number {
    return detected.filter((d) => !d.slug).length
}
