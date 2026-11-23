import { describe, expect, it } from 'vitest'

import { compareIds, mergeById, pollDelay, reconcileNewest } from './poll'

describe('pollDelay', () => {
    it('is the base interval until something fails', () => {
        expect(pollDelay(4000, 0)).toBe(4000)
    })

    it('doubles per failure up to the ceiling multiple', () => {
        expect(pollDelay(4000, 1)).toBe(8000)
        expect(pollDelay(4000, 2)).toBe(16000)
        expect(pollDelay(4000, 9)).toBe(32000)
    })

    it('has no ceiling when the multiple is 0', () => {
        expect(pollDelay(1000, 10, 0)).toBe(1024000)
    })
})

describe('ids and merging', () => {
    it('orders decimal ids past 2^53 numerically', () => {
        expect(compareIds('9007199254740993', '9007199254740992')).toBe(1)
        expect(compareIds('99', '100')).toBe(-1)
    })

    it('replaces a held row by id and keeps the order', () => {
        const held = [
            { id: '1', body: 'a' },
            { id: '2', body: 'b' },
        ]

        const out = mergeById(held, [{ id: '2', body: 'b (edited)' }, { id: '3', body: 'c' }], (a, b) =>
            compareIds(a.id, b.id)
        )

        expect(out.map((m) => m.body)).toEqual(['a', 'b (edited)', 'c'])
    })

    it('returns the same array when nothing arrived', () => {
        const held = [{ id: '1' }]

        expect(mergeById(held, [], (a, b) => compareIds(a.id, b.id))).toBe(held)
    })
})

describe('reconcileNewest', () => {
    const cmp = (a: { id: string }, b: { id: string }) => compareIds(a.id, b.id)
    const row = (id: string, body = id) => ({ id, body })

    it('replaces changed rows and drops ones deleted inside the page range', () => {
        const held = [row('1'), row('2'), row('3'), row('4')]
        const out = reconcileNewest(held, [row('2', 'two!'), row('4')], true, cmp)

        expect(out).toEqual([row('1'), row('2', 'two!'), row('4')])
    })

    it('keeps older history when the room has more', () => {
        expect(reconcileNewest([row('1'), row('5'), row('7')], [row('6'), row('8')], true, cmp).map((r) => r.id)).toEqual([
            '1',
            '5',
            '6',
            '8',
        ])
    })

    it('drops everything absent when the page is the whole room', () => {
        expect(reconcileNewest([row('1'), row('2')], [row('2')], false, cmp)).toEqual([row('2')])
        expect(reconcileNewest([row('1')], [], false, cmp)).toEqual([])
    })

    it('keeps held rows when an empty page says there is more', () => {
        expect(reconcileNewest([row('1')], [], true, cmp)).toEqual([row('1')])
    })
})
