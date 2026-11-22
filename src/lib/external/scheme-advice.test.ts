import { describe, expect, it } from 'vitest'

import { adviseScheme, planSchemeChanges, recommendedSchemes } from './scheme-advice'
import type { SchemeStatusT } from './schemas'

const free: SchemeStatusT = { state: 'free' }
const theirs: SchemeStatusT = { state: 'registered', by: 'Vortex', ours: false }
const ours: SchemeStatusT = { state: 'registered', by: 'GameSpice', ours: true }
const unknown: SchemeStatusT = { state: 'unknown' }

describe('scheme advice', () => {
    it('recommends only free schemes', () => {
        expect(adviseScheme('nxm', free).kind).toBe('recommend')
        expect(adviseScheme('nxm', ours).kind).toBe('ours')
        expect(adviseScheme('nxm', unknown).kind).toBe('unavailable')

        const r = adviseScheme('nxm', theirs)

        expect(r.kind).toBe('optIn')
        expect(r.kind === 'optIn' && r.warning).toContain('replace')
        expect(r.kind === 'optIn' && r.warning).toContain('Vortex')
    })

    it('pre-ticks free schemes and never an owned one', () => {
        const reports = [
            { scheme: 'nxm', status: theirs },
            { scheme: 'modrinth', status: free },
            { scheme: 'ror2mm', status: unknown },
        ]

        expect(recommendedSchemes(reports)).toEqual(['modrinth'])
    })

    it('never overrides without the explicit second tick', () => {
        const reports = [
            { scheme: 'nxm', status: theirs },
            { scheme: 'modrinth', status: free },
        ]
        const wanted = new Set(['nxm', 'modrinth'])

        expect(planSchemeChanges(reports, wanted, new Set())).toEqual([
            { scheme: 'modrinth', enable: true, overrideExisting: false },
        ])
        expect(planSchemeChanges(reports, wanted, new Set(['nxm']))).toContainEqual({
            scheme: 'nxm',
            enable: true,
            overrideExisting: true,
        })
    })

    it('turns off only what is ours, and leaves the rest alone', () => {
        const reports = [
            { scheme: 'nxm', status: ours },
            { scheme: 'modrinth', status: theirs },
        ]

        expect(planSchemeChanges(reports, new Set(), new Set())).toEqual([
            { scheme: 'nxm', enable: false, overrideExisting: false },
        ])
    })
})
