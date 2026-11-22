import { describe, expect, it } from 'vitest'

import type { ContentSummaryT } from '~/lib/api/contract'
import {
    MAX_FAVORITES,
    collectFavoritePages,
    favoritesFromRows,
    pickFavorites,
    filterFavorites,
    parseFavorites,
    refreshFavorites,
    toggleFavorite,
} from './favorites'

function srv(
    id: number,
    over: Partial<ContentSummaryT> = {},
    s: object = {}
): ContentSummaryT {
    return {
        id,
        name: `Server ${id}`,
        isOfficial: false,
        app: { id: 1, name: 'G' },
        server: { online: true, curUsers: 5, maxUsers: 10, ...s },
        ...over,
    } as unknown as ContentSummaryT
}

describe('toggleFavorite', () => {
    it('adds then removes without mutating', () => {
        const a = toggleFavorite({}, srv(1))

        expect(Object.keys(a)).toEqual(['1'])

        const b = toggleFavorite(a, srv(1))

        expect(b).toEqual({})
        expect(Object.keys(a)).toEqual(['1'])
    })

    it('caps at MAX_FAVORITES dropping the oldest', () => {
        let map = {}

        for (let i = 1; i <= MAX_FAVORITES + 2; i++)
            map = toggleFavorite(map, srv(i))

        expect(Object.keys(map)).toHaveLength(MAX_FAVORITES)
        expect('1' in map).toBe(false)
    })
})

describe('refreshFavorites', () => {
    it('updates only existing ids', () => {
        const map = toggleFavorite({}, srv(1))
        const next = refreshFavorites(map, [srv(1, { name: 'New' }), srv(2)])

        expect(next['1']?.name).toBe('New')
        expect('2' in next).toBe(false)
    })
})

describe('filterFavorites', () => {
    const list = [
        srv(1, { name: 'Alpha' }, { curUsers: 0 }),
        srv(2, { name: 'Beta' }, { curUsers: 10, maxUsers: 10 }),
        srv(3, { name: 'Gamma', app: { id: 2, name: 'H' } } as never, {
            online: false,
        }),
    ]

    it('searches by name', () => {
        expect(filterFavorites(list, { search: 'alp' }).map((x) => x.id)).toEqual([
            1,
        ])
    })

    it('hides empty, full and offline', () => {
        expect(filterFavorites(list, { hideEmpty: true }).map((x) => x.id)).toEqual(
            [2, 3]
        )
        expect(filterFavorites(list, { hideFull: true }).map((x) => x.id)).toEqual([
            1, 3,
        ])
        expect(
            filterFavorites(list, { onlineOnly: true }).map((x) => x.id)
        ).toEqual([1, 2])
    })

    it('filters by game', () => {
        expect(filterFavorites(list, { appId: 2 }).map((x) => x.id)).toEqual([3])
    })
})

describe('parseFavorites', () => {
    it('survives garbage', () => {
        expect(parseFavorites('nope')).toEqual({})
        expect(parseFavorites('[1]')).toEqual({})
        expect(parseFavorites(null)).toEqual({})
        expect(Object.keys(parseFavorites('{"1":{},"x":{}}'))).toEqual(['1'])
    })
})

describe('pickFavorites', () => {
    const local = { '1': srv(1) }

    it('uses the local map when signed out', () => {
        expect(pickFavorites(local, null)).toBe(local)
    })

    it('uses only the account set when signed in, never the local map', () => {
        const remote = { '2': srv(2) }

        expect(pickFavorites(local, remote)).toBe(remote)
        expect(pickFavorites(local, {})).toEqual({})
    })
})

describe('favoritesFromRows', () => {
    it('keys rows by id', () => {
        const map = favoritesFromRows([srv(3), srv(1)])

        expect(Object.keys(map).sort()).toEqual(['1', '3'])
    })
})

describe('collectFavoritePages', () => {
    it('follows cursors across every page', async () => {
        const pages: Record<string, { items: ContentSummaryT[]; nextCursor: string | null }> = {
            start: { items: [srv(1), srv(2)], nextCursor: 'b' },
            b: { items: [srv(3)], nextCursor: null },
        }
        const seen: (string | null)[] = []
        const rows = await collectFavoritePages((cursor) => {
            seen.push(cursor)

            return Promise.resolve(pages[cursor ?? 'start']!)
        })

        expect(rows.map((r) => r.id)).toEqual([1, 2, 3])
        expect(seen).toEqual([null, 'b'])
    })

    it('stops on a repeated cursor and at the page bound', async () => {
        let calls = 0
        const rows = await collectFavoritePages(() => {
            calls++

            return Promise.resolve({ items: [srv(calls)], nextCursor: 'same' })
        })

        expect(calls).toBe(2)
        expect(rows).toHaveLength(2)

        calls = 0
        await collectFavoritePages(() => {
            calls++

            return Promise.resolve({ items: [], nextCursor: `c${calls}` })
        }, 5)

        expect(calls).toBe(5)
    })
})

describe('toggleFavorite cap', () => {
    it('is unbounded when the cap is Infinity (account set)', () => {
        let map: Record<string, ContentSummaryT> = {}

        for (let i = 1; i <= MAX_FAVORITES + 5; i++)
            map = { ...toggleFavorite(map, srv(i), Infinity) }

        expect(Object.keys(map)).toHaveLength(MAX_FAVORITES + 5)
    })
})
