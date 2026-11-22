import { describe, expect, it } from 'vitest'

import { describeSize, extractLinks, isEditable } from './links'

describe('links', () => {
    it('finds links in prose and trims trailing punctuation', () => {
        expect(extractLinks('try https://modrinth.com/mod/sodium, or nxm://skyrim/mods/1/files/2.')).toEqual([
            'https://modrinth.com/mod/sodium',
            'nxm://skyrim/mods/1/files/2',
        ])
        expect(extractLinks('just words')).toEqual([])
        expect(extractLinks('javascript://x and file:///etc/passwd')).toEqual([])
    })

    it('leaves text fields alone', () => {
        expect(isEditable({ tagName: 'INPUT' } as unknown as EventTarget)).toBe(true)
        expect(isEditable({ tagName: 'DIV', isContentEditable: true } as unknown as EventTarget)).toBe(true)
        expect(isEditable({ tagName: 'DIV' } as unknown as EventTarget)).toBe(false)
        expect(isEditable(null)).toBe(false)
    })

    it('describes sizes', () => {
        expect(describeSize(null)).toBe('unknown size')
        expect(describeSize(2048)).toBe('2 KB')
        expect(describeSize(3 * 1024 * 1024)).toBe('3.0 MB')
    })
})
