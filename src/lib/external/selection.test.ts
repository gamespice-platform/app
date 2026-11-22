import { describe, expect, it } from 'vitest'

import type { FileTreeT } from './schemas'
import {
    allFolders,
    applyPreset,
    buildIndex,
    formatBytes,
    includedPaths,
    invert,
    PRESETS,
    selectAll,
    stateOf,
    summarise,
    toggle,
    visibleRows,
} from './selection'

const n = (path: string, isDir: boolean, size = 0) => ({
    path,
    name: path.split('/').pop() ?? path,
    isDir,
    size,
    depth: path.split('/').length - 1,
})

export const TREE: FileTreeT = {
    nodes: [
        n('plugins', true),
        n('plugins/a.dll', false, 2048),
        n('plugins/b.dll', false, 100),
        n('docs', true),
        n('docs/readme.txt', false, 10),
        n('docs/deep', true),
        n('docs/deep/x.png', false, 5000),
        n('top.jar', false, 7),
    ],
    suggested: [],
    totalFiles: 5,
    totalBytes: 7165,
    truncated: false,
}

const idx = buildIndex(TREE)
const node = (p: string) => idx.byPath.get(p)!

describe('selection', () => {
    it('computes tri-state folders from their files', () => {
        const c = new Set(['plugins/a.dll'])

        expect(stateOf(idx, c, node('plugins'))).toBe('some')
        expect(stateOf(idx, new Set(['plugins/a.dll', 'plugins/b.dll']), node('plugins'))).toBe('all')
        expect(stateOf(idx, c, node('docs'))).toBe('none')
    })

    it('toggling a folder takes everything under it, and again clears it', () => {
        const on = toggle(idx, new Set(), node('docs'))

        expect([...on].sort()).toEqual(['docs/deep/x.png', 'docs/readme.txt'])
        expect(toggle(idx, on, node('docs')).size).toBe(0)
        // A partly ticked folder fills up first.
        expect(toggle(idx, new Set(['docs/readme.txt']), node('docs')).size).toBe(2)
    })

    it('presets pick by extension and by folder, and invert flips', () => {
        const dll = applyPreset(idx, PRESETS.find((p) => p.id === 'binaries')!)

        expect([...dll].sort()).toEqual(['plugins/a.dll', 'plugins/b.dll', 'top.jar'])
        expect([...applyPreset(idx, PRESETS.find((p) => p.id === 'plugins')!)].sort()).toEqual(['plugins/a.dll', 'plugins/b.dll'])
        expect([...invert(idx, dll)].sort()).toEqual(['docs/deep/x.png', 'docs/readme.txt'])
        expect(selectAll(idx).size).toBe(5)
    })

    it('hands over the smallest path set', () => {
        expect(includedPaths(idx, selectAll(idx)).sort()).toEqual(['docs', 'plugins', 'top.jar'])
        expect(includedPaths(idx, new Set(['plugins/a.dll', 'docs/deep/x.png', 'docs/readme.txt'])).sort()).toEqual([
            'docs',
            'plugins/a.dll',
        ])
        expect(includedPaths(idx, new Set())).toEqual([])
    })

    it('lists rows folders-first, honouring open folders', () => {
        const sort = { key: 'name', dir: 'asc' } as const
        const closed = visibleRows(idx, new Set(), '', sort).map((r) => r.node.path)

        expect(closed).toEqual(['docs', 'plugins', 'top.jar'])

        const open = visibleRows(idx, allFolders(idx), '', sort).map((r) => r.node.path)

        expect(open).toEqual(['docs', 'docs/deep', 'docs/deep/x.png', 'docs/readme.txt', 'plugins', 'plugins/a.dll', 'plugins/b.dll', 'top.jar'])
    })

    it('search keeps matches with their ancestors and opens them', () => {
        const rows = visibleRows(idx, new Set(), 'X.PNG', { key: 'name', dir: 'asc' })

        expect(rows.map((r) => r.node.path)).toEqual(['docs', 'docs/deep', 'docs/deep/x.png'])
        expect(rows[0]?.open).toBe(true)
    })

    it('sorts by size descending with folders still first', () => {
        const rows = visibleRows(idx, new Set(['plugins']), '', { key: 'size', dir: 'desc' })

        expect(rows.map((r) => r.node.path)).toEqual(['docs', 'plugins', 'plugins/a.dll', 'plugins/b.dll', 'top.jar'])
    })

    it('totals and formats sizes', () => {
        expect(summarise(idx, new Set(['plugins/a.dll', 'top.jar']))).toEqual({ files: 2, bytes: 2055, total: 5 })
        expect(formatBytes(512)).toBe('512 B')
        expect(formatBytes(2048)).toBe('2.0 KB')
        expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
    })
})
