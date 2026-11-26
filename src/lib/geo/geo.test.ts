import { describe, expect, it } from 'vitest'

import { formatBytes, formatEstimate, progressPercent } from './geo'

describe('geo estimate formatting', () => {
    it('labels an estimate so it cannot pass for a ping', () => {
        expect(formatEstimate(44.6)).toBe('~45 ms est.')
        expect(formatEstimate(7)).toBe('~7 ms est.')
    })

    it('shows nothing for an unknown value', () => {
        expect(formatEstimate(null)).toBeNull()
        expect(formatEstimate(undefined)).toBeNull()
        expect(formatEstimate(Number.NaN)).toBeNull()
    })

    it('formats sizes and progress', () => {
        expect(formatBytes(512)).toBe('512 B')
        expect(formatBytes(3 * 1024 * 1024)).toBe('3.0 MB')
        expect(progressPercent(50, 200)).toBe(25)
        expect(progressPercent(5, null)).toBeNull()
        expect(progressPercent(500, 200)).toBe(100)
    })
})
