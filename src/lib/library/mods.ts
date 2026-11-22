import type { LibraryRowT, LocalModT } from '~/lib/ipc/schemas'

/**
 * Library → Mods: one row per mod on this device, whatever put it there.
 *
 * Two sources, both already on the device, so the page makes no request:
 *
 *   * **Subscribed**: library rows (`library_list`) that are installed, or
 *     disabled (paused) so they can be turned back on. A collection is not a
 *     row: its members are.
 *   * **Imported**: local mods (`local_list`), the files somebody dropped in or
 *     adopted from another mod manager.
 *
 * Every filter is a pure function here; the screen keeps them in the URL.
 */

export type ModSource = 'subscribed' | 'local'

export type ModRow = {
    key: string
    name: string
    kind: 'mod' | 'asset'
    appId: number | null
    appName: string | null
    source: ModSource
    /** A subscription that is not paused, or any imported mod. */
    enabled: boolean
    update: boolean
    version: string | null
    installedAt: string | null
    bytes: number | null
    sub: LibraryRowT | null
    local: LocalModT | null
}

export const MOD_STATUSES = ['all', 'enabled', 'disabled', 'updates'] as const
export type ModStatus = (typeof MOD_STATUSES)[number]

export const MOD_SOURCES = ['all', 'subscribed', 'local'] as const
export type ModSourceFilter = (typeof MOD_SOURCES)[number]

export const MOD_SORTS = ['name', 'game', 'recent', 'size'] as const
export type ModSort = (typeof MOD_SORTS)[number]

export function pick<T extends string>(values: readonly T[], raw: string | null, fallback: T): T {
    return values.find((v) => v === raw) ?? fallback
}

export function buildModRows(subs: readonly LibraryRowT[], locals: readonly LocalModT[]): ModRow[] {
    const names = new Map<number, string>()

    for (const r of subs) if (r.appId !== null && r.appName) names.set(r.appId, r.appName)

    const out: ModRow[] = []

    for (const r of subs) {
        if (r.kind === 'collection' || r.isContainer) continue
        if (r.installedReleaseId === null && !r.paused) continue

        out.push({
            key: `sub:${r.id}`,
            name: r.name,
            kind: r.kind,
            appId: r.appId,
            appName: r.appName,
            source: 'subscribed',
            enabled: !r.paused,
            update: r.updateAvailable,
            version: r.installedVersion,
            installedAt: r.installedAt,
            bytes: r.fileSize,
            sub: r,
            local: null,
        })
    }

    for (const m of locals)
        out.push({
            key: `local:${m.id}`,
            name: m.name,
            kind: 'mod',
            appId: m.appId,
            appName: (m.appId !== null ? names.get(m.appId) : undefined) ?? m.appSlug,
            source: 'local',
            enabled: true,
            update: false,
            version: m.version,
            installedAt: m.addedAt,
            bytes: m.bytes,
            sub: null,
            local: m,
        })

    return out
}

export type ModFilters = {
    q: string
    game: number | null
    status: ModStatus
    source: ModSourceFilter
    sort: ModSort
}

export function matchesMod(row: ModRow, f: ModFilters): boolean {
    const needle = f.q.trim().toLowerCase()

    if (needle && !row.name.toLowerCase().includes(needle)) return false
    if (f.game !== null && row.appId !== f.game) return false
    if (f.source !== 'all' && row.source !== f.source) return false

    switch (f.status) {
        case 'enabled':
            return row.enabled
        case 'disabled':
            return !row.enabled
        case 'updates':
            return row.update
        default:
            return true
    }
}

export function sortMods(rows: readonly ModRow[], sort: ModSort): ModRow[] {
    const byName = (a: ModRow, b: ModRow) => a.name.localeCompare(b.name)
    const cmp: Record<ModSort, (a: ModRow, b: ModRow) => number> = {
        name: byName,
        game: (a, b) => (a.appName ?? '￿').localeCompare(b.appName ?? '￿') || byName(a, b),
        recent: (a, b) => (b.installedAt ?? '').localeCompare(a.installedAt ?? '') || byName(a, b),
        size: (a, b) => (b.bytes ?? -1) - (a.bytes ?? -1) || byName(a, b),
    }

    return [...rows].sort(cmp[sort])
}

/** The games the rows belong to, for the game filter, by name. */
export function modGames(rows: readonly ModRow[]): { id: number; name: string }[] {
    const games = new Map<number, string>()

    for (const r of rows) if (r.appId !== null && !games.has(r.appId)) games.set(r.appId, r.appName ?? `Game #${r.appId}`)

    return [...games].map(([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name))
}

export function filterMods(rows: readonly ModRow[], f: ModFilters): ModRow[] {
    return sortMods(
        rows.filter((r) => matchesMod(r, f)),
        f.sort
    )
}
