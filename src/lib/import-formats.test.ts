import { describe, expect, it } from 'vitest'

import { archiveList, looksLikeArchive } from './import-formats'

describe('import formats', () => {
    it('lists 7z beside the other archives', () => {
        expect(archiveList()).toContain('.7z')
    })

    it.each(['a.zip', 'A.7Z', 'pack.mrpack', 'x.tar.gz', 'y.tgz'])('%s is an archive', (n) => {
        expect(looksLikeArchive(n)).toBe(true)
    })

    it.each(['a.jar', 'b.dll', 'zip', 'notes.txt'])('%s is not', (n) => {
        expect(looksLikeArchive(n)).toBe(false)
    })
})
