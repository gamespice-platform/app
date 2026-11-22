import { afterEach, describe, expect, it, vi } from 'vitest'
import { getConnectNoAsk, setConnectNoAsk } from './connect-preference'

function fakeStorage(): Storage {
    const m = new Map<string, string>()
    return {
        getItem: (k: string) => m.get(k) ?? null,
        setItem: (k: string, v: string) => void m.set(k, v),
        removeItem: (k: string) => void m.delete(k),
    } as unknown as Storage
}

afterEach(() => vi.unstubAllGlobals())

describe('connect-preference', () => {
    it('defaults to false, sets, and removes per game', () => {
        vi.stubGlobal('localStorage', fakeStorage())
        expect(getConnectNoAsk(1)).toBe(false)
        setConnectNoAsk(1, true)
        expect(getConnectNoAsk(1)).toBe(true)
        expect(getConnectNoAsk(2)).toBe(false)
        setConnectNoAsk(1, false)
        expect(getConnectNoAsk(1)).toBe(false)
    })

    it('treats throwing storage as "ask"', () => {
        const boom = () => {
            throw new Error('blocked')
        }
        vi.stubGlobal('localStorage', { getItem: boom, setItem: boom, removeItem: boom })
        expect(getConnectNoAsk(1)).toBe(false)
        expect(() => setConnectNoAsk(1, true)).not.toThrow()
        expect(() => setConnectNoAsk(1, false)).not.toThrow()
    })
})
