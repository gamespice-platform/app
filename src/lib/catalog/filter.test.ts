import { describe, expect, it } from 'vitest'

import type { CatalogueRow } from '~/lib/gamespice/catalogue'
import {
    CATALOG_TABS,
    PLATFORM_SLUGS,
    buildEntries,
    countTabs,
    matchesQuery,
    matchesTab,
    tabFromParam,
    type CatalogAppRow,
} from './filter'
import { parseManaged, toggleManaged } from './managed'

const app = (id: number, name: string, over: Partial<CatalogAppRow> = {}): CatalogAppRow => ({
    id,
    name,
    slug: name.toLowerCase().replace(/ /g, '-'),
    type: 'GAME',
    hasServers: false,
    ...over,
})

const play = (id: number, over: Partial<CatalogueRow> = {}): CatalogueRow => ({
    id,
    name: `p${id}`,
    url: null,
    solo: false,
    serverCnt: 0,
    lobbyCnt: 0,
    playerCnt: 0,
    isEngine: false,
    ...over,
})

describe('catalog tabs', () => {
    const minecraft = app(1, 'Minecraft', { hasServers: true })
    const portal = app(2, 'Portal', { type: 'GAME_SINGLEPLAYER' })
    const airdrift = app(3, 'Airdrift')
    const blender = app(4, 'Blender', { type: 'OTHER' })
    const entries = buildEntries(
        [airdrift],
        [minecraft, portal, airdrift, blender],
        [play(3, { solo: true, serverCnt: 2, url: 'airdrift' })],
        new Set([1]),
        new Set([2, 3])
    )

    it('puts GameSpice games first, once each, and auto-manages them', () => {
        expect(entries.map((e) => e.app.id)).toEqual([3, 1, 2, 4])
        expect(entries[0]).toMatchObject({ gamespice: true, managed: true, installed: true })
        expect(entries[0]?.play?.serverCnt).toBe(2)
    })

    it('filters each tab', () => {
        const ids = (tab: (typeof CATALOG_TABS)[number]) =>
            entries.filter((e) => matchesTab(e, tab)).map((e) => e.app.id)

        expect(ids('all')).toEqual([3, 1, 2, 4])
        expect(ids('managed')).toEqual([3, 1])
        expect(ids('unmanaged')).toEqual([2, 4])
        expect(ids('gamespice')).toEqual([3])
        expect(ids('installed')).toEqual([3, 2])
        expect(ids('multiplayer')).toEqual([3, 1])
        expect(ids('singleplayer')).toEqual([3, 2])
        expect(countTabs(entries)).toMatchObject({ all: 4, managed: 2, singleplayer: 2 })
    })

    it('recognises a GameSpice game by a former slug or by its play row', () => {
        const [a, b] = buildEntries(
            [],
            [app(9, 'G2G', { slug: '/g2gfast' }), app(10, 'Other', { slug: null })],
            [play(10, { url: 'prop-hunt' })],
            new Set(),
            new Set()
        )

        expect(a?.gamespice).toBe(true)
        expect(b?.gamespice).toBe(true)
        expect(PLATFORM_SLUGS).toContain('g2gfast')
    })

    it('reads the tab from the URL, defaulting to all', () => {
        expect(tabFromParam('installed')).toBe('installed')
        expect(tabFromParam('bogus')).toBe('all')
        expect(tabFromParam(null)).toBe('all')
    })

    it('narrows pinned rows by search and type on the client', () => {
        expect(matchesQuery(airdrift, 'air', null)).toBe(true)
        expect(matchesQuery(airdrift, 'zzz', null)).toBe(false)
        expect(matchesQuery(airdrift, '', 'GAME_SINGLEPLAYER')).toBe(false)
    })
})

describe('managed store', () => {
    it('toggles without mutating', () => {
        const a = new Set([1])
        const b = toggleManaged(a, 2)

        expect([...a]).toEqual([1])
        expect([...b]).toEqual([1, 2])
        expect([...toggleManaged(b, 1)]).toEqual([2])
    })

    it('parses tolerantly', () => {
        expect([...parseManaged('[1,2,"x",-1,3.5]')]).toEqual([1, 2])
        expect(parseManaged('{').size).toBe(0)
        expect(parseManaged(null).size).toBe(0)
    })
})
