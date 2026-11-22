import { describe, expect, it } from 'vitest'

import { DEFAULT_LAYOUT, LAYOUT_KEY, loadLayout, saveLayout } from './layout-prefs'

const memory = (initial?: string) => {
    const data = new Map<string, string>(initial === undefined ? [] : [[LAYOUT_KEY, initial]])

    return { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) }
}

describe('layout prefs', () => {
    it('round-trips', () => {
        const s = memory()
        const mine = { sort: { key: 'size', dir: 'desc' }, columns: { size: false, type: true } } as const

        saveLayout(mine, s)
        expect(loadLayout(s)).toEqual(mine)
    })

    it('falls back on missing, garbled or wrong-shaped data', () => {
        expect(loadLayout(memory())).toEqual(DEFAULT_LAYOUT)
        expect(loadLayout(memory('{nope'))).toEqual(DEFAULT_LAYOUT)
        expect(loadLayout(memory('{"sort":{"key":"colour","dir":"asc"}}'))).toEqual(DEFAULT_LAYOUT)
    })

    it('survives a store that throws', () => {
        const bad = {
            getItem: () => {
                throw new Error('denied')
            },
            setItem: () => {
                throw new Error('full')
            },
        }

        expect(loadLayout(bad)).toEqual(DEFAULT_LAYOUT)
        expect(() => saveLayout(DEFAULT_LAYOUT, bad)).not.toThrow()
    })
})
