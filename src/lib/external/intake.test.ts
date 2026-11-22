import { describe, expect, it } from 'vitest'

import { confirmFacts, IDLE, reduce } from './intake'

const pending = {
    kind: 'confirm' as const,
    provider: 'modrinth' as const,
    label: 'Modrinth',
    url: 'https://modrinth.com/mod/sodium',
    reference: { provider: 'modrinth' as const, project: 'sodium', version: null, game: 'minecraft', class: null, url: 'https://modrinth.com/mod/sodium', extras: {} },
}

describe('intake reducer', () => {
    it('walks a link through to done and back to idle', () => {
        let s = reduce(IDLE, { type: 'start', step: 'parsing', label: 'x' })

        expect(s.phase).toBe('working')
        s = reduce(s, { type: 'done', localId: 4, name: 'Sodium', files: 1, sandboxId: null })
        expect(s).toMatchObject({ phase: 'done', localId: 4 })
        expect(reduce(s, { type: 'dismiss' })).toEqual(IDLE)
    })

    it('a deep link never interrupts work in progress or a picker', () => {
        const working = reduce(IDLE, { type: 'start', step: 'installing', label: 'x' })

        expect(reduce(working, { type: 'confirm', pending })).toBe(working)
        expect(reduce(IDLE, { type: 'confirm', pending }).phase).toBe('confirm')
    })

    it('shows provider, project, version and game before the lookup', () => {
        expect(confirmFacts(pending)).toEqual([
            { label: 'Provider', value: 'Modrinth' },
            { label: 'Project', value: 'sodium' },
            { label: 'Version', value: 'Latest' },
            { label: 'Game', value: 'minecraft' },
        ])
    })
})
