import { describe, expect, it } from 'vitest'

import type { DownloadT } from '~/lib/ipc/schemas'

import { canFinalize, finalizeSummary, packProgress, packRows } from './local-pack'

function row(over: Partial<DownloadT>): DownloadT {
    return {
        id: 'pack7:0',
        label: 'x',
        url: 'https://cdn.modrinth.com/x',
        dest: '',
        status: 'queued',
        total: 100,
        done: 0,
        speedBps: 0,
        etaSecs: null,
        priority: 0,
        limitBps: null,
        attempts: 0,
        error: null,
        sha256: null,
        samples: [],
        queuedAt: '',
        updatedAt: '',
        meta: { pack: '7' },
        ...over,
    }
}

describe('packRows', () => {
    it('takes only rows tagged with that mod id', () => {
        const rows = [row({}), row({ id: 'a', meta: { pack: '8' } }), row({ id: 'b', meta: {} })]

        expect(packRows(rows, 7)).toHaveLength(1)
        expect(packRows(rows, 70)).toHaveLength(0)
    })
})

describe('packProgress', () => {
    it('is empty when nothing was queued', () => {
        const p = packProgress([], 7)

        expect(p.total).toBe(0)
        expect(p.fraction).toBe(0)
        expect(canFinalize(p)).toBe(false)
    })

    it('counts by bytes while totals are known', () => {
        const p = packProgress(
            [row({ status: 'done', done: 100 }), row({ id: 'pack7:1', status: 'running', done: 50 })],
            7
        )

        expect(p).toMatchObject({ total: 2, done: 1, open: 1, bytesDone: 150, bytesTotal: 200 })
        expect(p.fraction).toBeCloseTo(0.75)
        expect(canFinalize(p)).toBe(false)
    })

    it('falls back to file count when a total is unknown', () => {
        const p = packProgress(
            [row({ status: 'done' }), row({ id: 'pack7:1', status: 'running', total: null })],
            7
        )

        expect(p.bytesTotal).toBeNull()
        expect(p.fraction).toBe(0.5)
    })

    it('may finalize once every row has ended, failures included', () => {
        const p = packProgress(
            [row({ status: 'done' }), row({ id: 'pack7:1', status: 'failed' })],
            7
        )

        expect(p.failed).toBe(1)
        expect(canFinalize(p)).toBe(true)
    })
})

describe('finalizeSummary', () => {
    it('names pending and failed only when there are some', () => {
        expect(finalizeSummary({ placed: 3, pending: 0, failed: [] })).toBe('3 placed')
        expect(finalizeSummary({ placed: 1, pending: 2, failed: ['mods/a.jar'] })).toBe(
            '1 placed, 2 not downloaded yet, 1 failed the hash check and were removed'
        )
    })
})
