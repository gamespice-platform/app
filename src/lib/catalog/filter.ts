import type { AppTypeT } from '~/lib/api/contract'
import { isMultiplayer, isPlatform, type CatalogueRow } from '~/lib/gamespice/catalogue'
import { PLATFORM_GAMES, isPlatformSlug } from '~/lib/gamespice/games'

/**
 * The Apps catalogue's tabs, as pure functions over one merged row per app.
 *
 * Two sources describe a game: the `/apps` catalogue row (type, counts, install
 * builds) and, for games the play centre lists, the `/play/page` row (live
 * players, servers, the 24h history, solo). `buildEntries` joins them by id and
 * adds the device facts (installed, managed); the tabs then read only the entry.
 */

export const CATALOG_TABS = [
    'all',
    'managed',
    'unmanaged',
    'gamespice',
    'installed',
    'multiplayer',
    'singleplayer',
] as const

export type CatalogTab = (typeof CATALOG_TABS)[number]

export const CATALOG_TAB_LABELS: Record<CatalogTab, string> = {
    all: 'All',
    managed: 'Managed',
    unmanaged: 'Unmanaged',
    gamespice: 'GameSpice',
    installed: 'Installed',
    multiplayer: 'Multiplayer',
    singleplayer: 'Singleplayer',
}

/** Every slug (current and former) of GameSpice's own games, for one lookup. */
export const PLATFORM_SLUGS: readonly string[] = PLATFORM_GAMES.flat()

export function tabFromParam(raw: string | null): CatalogTab {
    return CATALOG_TABS.find((t) => t === raw) ?? 'all'
}

/** The fields of an `/apps` row the tabs read. */
export type CatalogAppRow = {
    id: number
    name: string
    slug: string | null
    type: AppTypeT
    hasServers: boolean
}

export type CatalogEntry<A extends CatalogAppRow, P extends CatalogueRow> = {
    app: A
    /** The play centre's row for the same game, when it lists it. */
    play: P | null
    gamespice: boolean
    managed: boolean
    installed: boolean
}

/**
 * One entry per app id, first wins. `pinned` (GameSpice's own games, fetched by
 * slug) lead, then `apps` in the server's order, so the user's sort still
 * decides among the rest. GameSpice games are always managed.
 */
export function buildEntries<A extends CatalogAppRow, P extends CatalogueRow>(
    pinned: readonly A[],
    apps: readonly A[],
    playRows: readonly P[],
    managed: ReadonlySet<number>,
    installed: ReadonlySet<number>
): CatalogEntry<A, P>[] {
    const plays = new Map<number, P>()

    for (const row of playRows) if (!plays.has(row.id)) plays.set(row.id, row)

    const seen = new Set<number>()
    const out: CatalogEntry<A, P>[] = []

    for (const app of [...pinned, ...apps]) {
        if (seen.has(app.id)) continue
        seen.add(app.id)

        const play = plays.get(app.id) ?? null
        const gamespice = isPlatformSlug(app.slug) || (play !== null && isPlatform(play))

        out.push({
            app,
            play,
            gamespice,
            managed: gamespice || managed.has(app.id),
            installed: installed.has(app.id),
        })
    }

    // Stable: GameSpice first, everything else keeps its order.
    return [...out.filter((e) => e.gamespice), ...out.filter((e) => !e.gamespice)]
}

export function entryIsMultiplayer(e: CatalogEntry<CatalogAppRow, CatalogueRow>): boolean {
    if (e.play) return isMultiplayer(e.play)

    return e.app.type === 'GAME' || e.app.hasServers
}

export function entryIsSingleplayer(e: CatalogEntry<CatalogAppRow, CatalogueRow>): boolean {
    return e.app.type === 'GAME_SINGLEPLAYER' || e.play?.solo === true
}

export function matchesTab(e: CatalogEntry<CatalogAppRow, CatalogueRow>, tab: CatalogTab): boolean {
    switch (tab) {
        case 'managed':
            return e.managed
        case 'unmanaged':
            return !e.managed
        case 'gamespice':
            return e.gamespice
        case 'installed':
            return e.installed
        case 'multiplayer':
            return entryIsMultiplayer(e)
        case 'singleplayer':
            return entryIsSingleplayer(e)
        default:
            return true
    }
}

export function countTabs(
    entries: readonly CatalogEntry<CatalogAppRow, CatalogueRow>[]
): Record<CatalogTab, number> {
    const counts = Object.fromEntries(CATALOG_TABS.map((t) => [t, 0])) as Record<CatalogTab, number>

    for (const e of entries) for (const t of CATALOG_TABS) if (matchesTab(e, t)) counts[t] += 1

    return counts
}

/**
 * The client-side half of a search for the pinned GameSpice rows, which are
 * fetched by slug and so never narrowed by the server's `search`/`type`.
 */
export function matchesQuery(app: CatalogAppRow, q: string, type: AppTypeT | null): boolean {
    const needle = q.trim().toLowerCase()

    return (!type || app.type === type) && (!needle || app.name.toLowerCase().includes(needle))
}
