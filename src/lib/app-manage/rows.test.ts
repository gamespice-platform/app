import { describe, expect, it } from 'vitest'

import type { DetectedGameT } from '~/lib/ipc/schemas'
import { buildManageRows, slugOf, unmatchedCount } from '~/lib/app-manage/rows'

const game = (id: number, name: string, url: string | null = name) => ({
    id,
    name,
    icon: null,
    url,
})

const found = (slug: string | null, path = '/g'): DetectedGameT => ({
    source: 'steam',
    name: 'x',
    path,
    launcherId: null,
    launchUri: null,
    sizeBytes: null,
    slug,
    alreadySet: false,
    replaces: null,
})

describe('slugOf', () => {
    it('lower-cases and rejects blank', () => {
        expect(slugOf({ url: 'GTAV' })).toBe('gtav')
        expect(slugOf({ url: '  ' })).toBeNull()
        expect(slugOf({ url: null })).toBeNull()
    })
})

describe('buildManageRows', () => {
    const catalogue = [game(1, 'Zed'), game(2, 'Alpha'), game(3, 'Mid')]

    it('orders installed, detected, other, then by name', () => {
        const rows = buildManageRows({
            catalogue,
            gameDirs: { '1': '/games/zed' },
            sandboxAppIds: [],
            detected: [found('mid')],
        })

        expect(rows.map((r) => [r.name, r.status])).toEqual([
            ['Zed', 'installed'],
            ['Mid', 'detected'],
            ['Alpha', 'other'],
        ])
    })

    it('counts a sandbox without a folder as installed', () => {
        const rows = buildManageRows({
            catalogue,
            gameDirs: {},
            sandboxAppIds: [2],
            detected: [],
        })

        expect(rows[0]).toMatchObject({
            name: 'Alpha',
            status: 'installed',
            dir: null,
            hasSandbox: true,
        })
    })

    it('keeps a folder for a game missing from the catalogue', () => {
        const rows = buildManageRows({
            catalogue: [],
            gameDirs: { '99': '/old', nope: '/x' },
            sandboxAppIds: [],
            detected: [],
        })

        expect(rows).toHaveLength(1)
        expect(rows[0]).toMatchObject({ name: 'Game #99', status: 'installed' })
    })

    it('matches scan results by slug, case-insensitively, and filters by search', () => {
        const rows = buildManageRows({
            catalogue,
            gameDirs: {},
            sandboxAppIds: [],
            detected: [found('ALPHA'), found(null)],
            search: 'alp',
        })

        expect(rows).toHaveLength(1)
        expect(rows[0]?.detected).toHaveLength(1)
    })
})

describe('unmatchedCount', () => {
    it('counts results with no slug', () => {
        expect(unmatchedCount([found(null), found('a'), found(null)])).toBe(2)
    })
})
