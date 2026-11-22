import { useEffect } from 'react'

import { api } from '~/lib/api/client'
import type { ContentSummaryT } from '~/lib/api/contract'
import {
    collectFavoritePages,
    enterAccountFavorites,
    favoritesFromRows,
    leaveAccountFavorites,
    revertRemoteFavorite,
    setRemoteFavorites,
    toggleStoredFavorite,
} from '~/lib/favorites'

/** Load every page of the account's favourite servers into the store. */
export async function loadAccountFavorites(): Promise<void> {
    const rows = await collectFavoritePages((cursor) =>
        api.browseFavorites({
            kind: 'server',
            favorites: true,
            cursor: cursor ?? undefined,
        })
    )

    setRemoteFavorites(favoritesFromRows(rows))
}

/**
 * Toggle a star: optimistic flip in the active source; signed in, also POST to
 * the account and roll the flip back if that fails.
 */
export async function toggleFavoriteItem(item: ContentSummaryT): Promise<void> {
    const { account, favorite } = toggleStoredFavorite(item)

    if (!account) return

    try {
        await api.setFavorite({ kind: 'server', id: Number(item.id), favorite })
    } catch {
        revertRemoteFavorite(item)
    }
}

/** Follows sign-in state: account source while signed in, local otherwise. */
export function useFavoritesSync(signedIn: boolean): void {
    useEffect(() => {
        if (!signedIn) {
            leaveAccountFavorites()

            return
        }

        enterAccountFavorites()
        loadAccountFavorites().catch(() => {
            // Offline: the account set stays as loaded (empty on first run).
        })
    }, [signedIn])
}
