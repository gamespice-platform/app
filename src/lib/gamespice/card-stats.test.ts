import { describe, expect, it } from 'vitest'

import { barHeight, cardStats } from './card-stats'

const h = (ts: string, players: number, serversOnline = 1) => ({ ts, players, serversOnline })

describe('cardStats', () => {
    it('finds the peak and its hour', () => {
        const s = cardStats([h('2026-11-20T10:00:00Z', 3), h('2026-11-20T11:00:00Z', 9), h('2026-11-20T12:00:00Z', 4)])
        expect(s.peak).toBe(9)
        expect(s.peakAt).toBe('2026-11-20T11:00:00Z')
        expect(s.hours).toHaveLength(3)
    })

    it('has no hours without history', () => {
        expect(cardStats(null)).toEqual({ hours: [], peak: 0, peakAt: null })
        expect(cardStats([h('2026-11-20T10:00:00Z', 0)]).peakAt).toBeNull()
    })
})

describe('barHeight', () => {
    it('is 0 for nobody, at least 8 for anybody, 100 at the peak', () => {
        expect(barHeight(0, 10)).toBe(0)
        expect(barHeight(1, 1000)).toBe(8)
        expect(barHeight(10, 10)).toBe(100)
    })
})
