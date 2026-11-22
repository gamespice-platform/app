import { useSyncExternalStore } from 'react'

import type { ContentSummaryT } from '~/lib/api/contract'

/**
 * Favourite servers: two sources, never merged.
 *
 * Signed in, the ACCOUNT is the single source of truth, exactly like the
 * website: the Favourites view lists `/browse?kind=server&favorites=1`, the
 * star state comes from that set, and a toggle POSTs `/favorites` after an
 * optimistic flip (rolled back if the POST fails). See `favorites-sync.ts`.
 *
 * Signed out (or never signed in), favourites live in localStorage as a
 * snapshot of the row (`ContentSummary`), which also lets the Favourites view
 * render offline. The local map is NOT merged into the account on sign-in and
 * the account is not copied down: the local map stays untouched on the device
 * for the next signed-out session. `pickFavorites` is the one place that
 * chooses between them.
 */

export const FAVORITES_KEY = 'tmc.favorites.servers'
export const MAX_FAVORITES = 500

export type FavoriteMap = Readonly<Record<string, ContentSummaryT>>

/** Pure toggle: add when absent, remove when present. Never mutates. */
export function toggleFavorite(
    map: FavoriteMap,
    item: ContentSummaryT,
    cap: number = MAX_FAVORITES
): FavoriteMap {
    const id = String(item.id)

    if (id in map) {
        const { [id]: _removed, ...rest } = map

        return rest
    }

    const next = { ...map, [id]: item }
    const keys = Object.keys(next)

    // Oldest-inserted first; drop the overflow rather than refusing to save.
    if (keys.length > cap) {
        for (const key of keys.slice(0, keys.length - cap))
            delete next[key]
    }

    return next
}

/** Replace snapshots for ids that are still favourites; never adds. */
export function refreshFavorites(
    map: FavoriteMap,
    fresh: ContentSummaryT[]
): FavoriteMap {
    const next: Record<string, ContentSummaryT> = { ...map }

    for (const item of fresh) {
        const id = String(item.id)

        if (id in next) next[id] = item
    }

    return next
}

/**
 * Which map the UI shows. `remote` is null when signed out; signed in it is the
 * account's set (empty until the first fetch lands, never the local map).
 */
export function pickFavorites(
    local: FavoriteMap,
    remote: FavoriteMap | null
): FavoriteMap {
    return remote ?? local
}

/** Build a map from account rows, in the order the API returned them. */
export function favoritesFromRows(rows: ContentSummaryT[]): FavoriteMap {
    const out: Record<string, ContentSummaryT> = {}

    for (const item of rows) out[String(item.id)] = item

    return out
}

export type FavoritePage = {
    items: ContentSummaryT[]
    nextCursor: string | null
}

/** Follow `nextCursor` until the end (bounded, so a looping cursor ends). */
export async function collectFavoritePages(
    fetchPage: (cursor: string | null) => Promise<FavoritePage>,
    maxPages = 200
): Promise<ContentSummaryT[]> {
    const rows: ContentSummaryT[] = []
    let cursor: string | null = null

    for (let i = 0; i < maxPages; i++) {
        const page: FavoritePage = await fetchPage(cursor)

        rows.push(...page.items)

        if (!page.nextCursor || page.nextCursor === cursor) break

        cursor = page.nextCursor
    }

    return rows
}

export type FavoriteFilters = {
    search?: string
    appId?: number
    onlineOnly?: boolean
    hideEmpty?: boolean
    hideFull?: boolean
    password?: boolean
    secure?: boolean
    isOfficial?: boolean
}

/** Client-side equivalent of the API filters the Favourites view can honour. */
export function filterFavorites(
    list: ContentSummaryT[],
    f: FavoriteFilters
): ContentSummaryT[] {
    const needle = f.search?.trim().toLowerCase()

    return list.filter((item) => {
        const s = item.server

        if (needle && !item.name.toLowerCase().includes(needle)) return false
        if (f.appId != null && item.app?.id !== f.appId) return false
        if (f.onlineOnly && s && !s.online) return false
        if (f.isOfficial && !item.isOfficial) return false
        if (f.hideEmpty && s && s.curUsers <= 0) return false
        if (f.hideFull && s && s.maxUsers > 0 && s.curUsers >= s.maxUsers)
            return false

        return true
    })
}

export function parseFavorites(raw: string | null): FavoriteMap {
    try {
        const value: unknown = JSON.parse(raw ?? '{}')

        if (typeof value !== 'object' || value === null || Array.isArray(value))
            return {}

        const out: Record<string, ContentSummaryT> = {}

        for (const [key, item] of Object.entries(value)) {
            if (/^\d+$/.test(key) && typeof item === 'object' && item !== null) {
                out[key] = item as ContentSummaryT
            }
        }

        return out
    } catch {
        return {}
    }
}

// ------------------------------------------------------------------ store

let current: FavoriteMap | null = null
/** The account's set while signed in; null when signed out. */
let remote: FavoriteMap | null = null
const listeners = new Set<() => void>()

function read(): FavoriteMap {
    if (current === null) {
        let raw: string | null = null

        try {
            raw =
                typeof localStorage === 'undefined'
                    ? null
                    : localStorage.getItem(FAVORITES_KEY)
        } catch {
            // Blocked storage: start empty rather than crash every render.
        }

        current = parseFavorites(raw)
    }

    return current
}

function emit(): void {
    for (const listener of listeners) listener()
}

function write(next: FavoriteMap): void {
    current = next

    try {
        localStorage.setItem(FAVORITES_KEY, JSON.stringify(next))
    } catch {
        // Blocked storage: favourites last until the window closes.
    }

    emit()
}

/** Signed in: switch to the account source (empty until `setRemoteFavorites`). */
export function enterAccountFavorites(): void {
    if (remote === null) {
        remote = {}
        emit()
    }
}

/** Signed out: back to the local source, which was never touched. */
export function leaveAccountFavorites(): void {
    if (remote !== null) {
        remote = null
        emit()
    }
}

export function setRemoteFavorites(next: FavoriteMap): void {
    remote = next
    emit()
}

/**
 * Flip a favourite in the active source. Returns whether the account is the
 * source (the caller then POSTs) and the new state.
 */
export function toggleStoredFavorite(item: ContentSummaryT): {
    account: boolean
    favorite: boolean
} {
    const id = String(item.id)

    if (remote !== null) {
        const favorite = !(id in remote)

        remote = toggleFavorite(remote, item, Infinity)
        emit()

        return { account: true, favorite }
    }

    const favorite = !(id in read())

    write(toggleFavorite(read(), item))

    return { account: false, favorite }
}

/** Undo an optimistic account flip after a failed POST. */
export function revertRemoteFavorite(item: ContentSummaryT): void {
    if (remote !== null) {
        remote = toggleFavorite(remote, item, Infinity)
        emit()
    }
}

/** Refresh local snapshots (signed out only; account rows are already fresh). */
export function refreshStoredFavorites(fresh: ContentSummaryT[]): void {
    write(refreshFavorites(read(), fresh))
}

function subscribe(listener: () => void): () => void {
    listeners.add(listener)

    return () => listeners.delete(listener)
}

function snapshot(): FavoriteMap {
    return pickFavorites(read(), remote)
}

/** The active favourites map; re-renders on any change (optimistic). */
export function useFavorites(): FavoriteMap {
    return useSyncExternalStore(subscribe, snapshot, snapshot)
}
