import { describe, expect, it } from 'vitest'

import { downloadInfo } from './download-info'

describe('downloadInfo', () => {
    it('describes an engine update with its users', () => {
        const info = downloadInfo({
            kind: 'game',
            role: 'engine',
            action: 'update',
            fromVersion: '1.1',
            usedBy: 'Fatal Contact, Airdrift',
            reason: 'automatic',
            version: '1.2',
            platform: 'linux-x64',
        })

        expect(info.subtitle).toBe(
            'Engine update from 1.1 · used by Fatal Contact, Airdrift'
        )
        expect(info.details.map((d) => d.label)).toEqual([
            'Version',
            'Previous version',
            'Platform',
            'Used by',
            'Started',
        ])
        expect(info.details.at(-1)?.value).toBe('Automatic update')
    })

    it('describes a fresh game install', () => {
        const info = downloadInfo({
            kind: 'game',
            role: 'game',
            action: 'install',
            reason: 'manual',
        })

        expect(info.subtitle).toBe('Game install')
        expect(info.details).toEqual([
            { label: 'Started', value: 'Manual request' },
        ])
    })

    it('says nothing for other rows', () => {
        expect(downloadInfo({ kind: 'mod' })).toEqual({
            subtitle: null,
            details: [],
        })
    })
})
