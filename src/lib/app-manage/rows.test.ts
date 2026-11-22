import { describe, expect, it } from 'vitest'

import type { DetectedGameT } from '~/lib/ipc/schemas'
import {
    buildManageRows,
    sameDir,
    slugOf,
    unmatchedCount,
    unplacedDetected,
    unsupportedDetected,
} from '~/lib/app-manage/rows'

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
    appId: null,
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

describe('scan results the catalogue facet cannot place', () => {
    const catalogue = [game(1, 'Zed')]
    const withId = (slug: string | null, appId: number | null, name = 'Found'): DetectedGameT => ({
        ...found(slug),
        appId,
        name,
    })

    it('matches by appId even when the catalogue url differs from the slug', () => {
        const rows = buildManageRows({
            catalogue: [game(1, 'Zed', 'zed-site')],
            gameDirs: {},
            sandboxAppIds: [],
            detected: [withId('zed', 1)],
        })

        expect(rows[0]).toMatchObject({ appId: 1, status: 'detected' })
        expect(rows[0]!.detected).toHaveLength(1)
    })

    it('adds a row for an appId the catalogue does not list, named from the scan', () => {
        const rows = buildManageRows({
            catalogue,
            gameDirs: {},
            sandboxAppIds: [],
            detected: [withId('other', 99, 'Other Game')],
        })

        expect(rows.find((r) => r.appId === 99)).toMatchObject({
            name: 'Other Game',
            status: 'detected',
        })
    })

    it('falls back to the slug when Rust sent no id', () => {
        const rows = buildManageRows({
            catalogue,
            gameDirs: {},
            sandboxAppIds: [],
            detected: [withId('zed', null)],
        })

        expect(rows[0]!.status).toBe('detected')
    })

    it('lists a slugged result with no id and no catalogue match as unplaced', () => {
        const detected = [withId('zed', null), withId('ghost', null), withId(null, null)]

        expect(unplacedDetected({ catalogue, detected }).map((d) => d.slug)).toEqual(['ghost'])
        expect(unsupportedDetected(detected)).toHaveLength(1)
        expect(unmatchedCount(detected)).toBe(1)
    })
})

describe('sameDir', () => {
    it('ignores a trailing slash, slash direction and drive-letter case', () => {
        expect(sameDir('/a/b/', '/a/b')).toBe(true)
        expect(sameDir('C:\\Games\\X\\', 'c:/Games/X')).toBe(true)
        expect(sameDir('/a/b', '/a/c')).toBe(false)
        expect(sameDir(null, '/a')).toBe(false)
    })
})
