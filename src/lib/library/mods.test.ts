import { describe, expect, it } from 'vitest'

import type { LibraryRowT, LocalModT } from '~/lib/ipc/schemas'

import { buildModRows, filterMods, modGames, pick, MOD_SORTS, type ModFilters } from './mods'

const sub = (over: Partial<LibraryRowT>): LibraryRowT => ({
    id: 'mod:1',
    kind: 'mod',
    itemId: 1,
    name: 'Sub',
    description: null,
    image: null,
    webUrl: 'https://example.invalid/m/1',
    appId: 30,
    appName: 'Minecraft',
    appSlug: 'minecraft',
    autoUpdate: true,
    notifyUpdates: true,
    paused: false,
    viaCollectionId: null,
    installable: true,
    latestReleaseId: 2,
    latestVersion: '2.0',
    fileUrl: null,
    fileName: null,
    fileSize: 100,
    fileSha256: null,
    updatedAt: '2026-01-01',
    installedReleaseId: 1,
    installedVersion: '1.0',
    installedInstallId: null,
    installedAt: '2026-01-02',
    installedFiles: [],
    state: 'installed',
    lastError: null,
    updateAvailable: false,
    hasRule: true,
    isContainer: false,
    ...over,
})

const local = (over: Partial<LocalModT>): LocalModT => ({
    id: 7,
    name: 'Local',
    appId: 4,
    appSlug: 'gmod',
    version: null,
    author: null,
    notes: null,
    origin: 'dropped',
    originLabel: null,
    relPath: 'addons/local',
    files: 1,
    bytes: 50,
    addedAt: '2026-01-03',
    updatedAt: '2026-01-03',
    ...over,
})

const F: ModFilters = { q: '', game: null, status: 'all', source: 'all', sort: 'name' }

const rows = buildModRows(
    [
        sub({ id: 'mod:1', name: 'Alpha', updateAvailable: true }),
        sub({ id: 'mod:2', name: 'Beta', paused: true, installedReleaseId: null, installedAt: null }),
        sub({ id: 'mod:3', name: 'Not installed', installedReleaseId: null }),
        sub({ id: 'collection:4', kind: 'collection', name: 'Pack', isContainer: true }),
    ],
    [local({ name: 'Gamma' })]
)

describe('Library → Mods rows', () => {
    it('keeps installed and disabled subscriptions plus imports, never collections', () => {
        expect(rows.map((r) => r.name)).toEqual(['Alpha', 'Beta', 'Gamma'])
        expect(rows.find((r) => r.name === 'Beta')?.enabled).toBe(false)
        expect(rows.find((r) => r.name === 'Gamma')?.source).toBe('local')
    })

    it('filters by status, source, game and search', () => {
        const names = (f: Partial<ModFilters>) => filterMods(rows, { ...F, ...f }).map((r) => r.name)

        expect(names({ status: 'updates' })).toEqual(['Alpha'])
        expect(names({ status: 'disabled' })).toEqual(['Beta'])
        expect(names({ status: 'enabled' })).toEqual(['Alpha', 'Gamma'])
        expect(names({ source: 'local' })).toEqual(['Gamma'])
        expect(names({ game: 30 })).toEqual(['Alpha', 'Beta'])
        expect(names({ q: ' gAm ' })).toEqual(['Gamma'])
    })

    it('sorts by game, recency and size', () => {
        const names = (sort: ModFilters['sort']) => filterMods(rows, { ...F, sort }).map((r) => r.name)

        expect(names('game')).toEqual(['Gamma', 'Alpha', 'Beta'])
        expect(names('recent')).toEqual(['Gamma', 'Alpha', 'Beta'])
        expect(names('size')).toEqual(['Alpha', 'Beta', 'Gamma'])
    })

    it('lists the games and rejects unknown URL values', () => {
        expect(modGames(rows)).toEqual([
            { id: 4, name: 'gmod' },
            { id: 30, name: 'Minecraft' },
        ])
        expect(pick(MOD_SORTS, 'bogus', 'game')).toBe('game')
        expect(pick(MOD_SORTS, 'size', 'game')).toBe('size')
    })
})
