import { describe, expect, it } from 'vitest'

import { sandboxGameOptions, sandboxSlugs } from './sandbox-games'

const app = (id: number, name: string, slug: string | null) => ({
    id,
    name,
    slug,
    images: { icon: null },
})

describe('New sandbox game list', () => {
    it('offers a supported game with no mods on the site (Minecraft)', () => {
        // The old source, `/facets?kind=mod`, only had games with published
        // mods; Minecraft (set up via Find games, zero site mods) was missing.
        const options = sandboxGameOptions(
            [app(4, "Garry's Mod", 'gmod'), app(30, 'Minecraft', 'minecraft'), app(5, 'Rust', 'rust')],
            ['rust', 'minecraft', 'gmod']
        )

        expect(options.map((o) => o.name)).toEqual(["Garry's Mod", 'Minecraft', 'Rust'])
    })

    it('drops apps without a manage rule and duplicates', () => {
        const options = sandboxGameOptions(
            [app(30, 'Minecraft', 'Minecraft'), app(30, 'Minecraft', 'minecraft'), app(6, 'CS:S', 'css'), app(7, 'X', null)],
            ['minecraft']
        )

        expect(options).toEqual([{ id: 30, name: 'Minecraft', slug: 'minecraft', icon: null }])
    })

    it('normalises the slug list', () => {
        expect(sandboxSlugs(['Rust', 'rust', ' gmod ', ''])).toEqual(['gmod', 'rust'])
    })
})
