import { describe, expect, it } from 'vitest'

import type { AppPartyMemberT } from '~/lib/api/contract'
import {
    BLANK_CREATE,
    buildCreateRequest,
    canManageRole,
    clampMinutes,
    clock,
    countdownRemaining,
    invitableFriends,
    joinedMembers,
    kdRatio,
    memberPowers,
    mergePages,
    readyPercent,
    roleLabel,
    stageLabel,
} from './logic'

const member = (over: Partial<AppPartyMemberT>): AppPartyMemberT => ({
    userId: 'u',
    displayName: 'x',
    gameName: null,
    role: 'MEMBER',
    state: 'JOINED',
    presence: 'ONLINE',
    joinedAt: '2026-01-01T00:00:00Z',
    leftAt: null,
    readyAt: null,
    connectedAt: null,
    score: 0,
    kills: 0,
    deaths: 0,
    assist: 0,
    ...over,
})

describe('labels', () => {
    it('names known values and passes new ones through readably', () => {
        expect(roleLabel('CO_HOST')).toBe('Co-host')
        expect(roleLabel('SOME_NEW_ROLE')).toBe('Some new role')
        expect(stageLabel('READY')).toBe('Ready check')
    })
})

describe('joinedMembers', () => {
    it('drops people who left, orders host, co-host, then by name', () => {
        const list = joinedMembers([
            member({ userId: '1', displayName: 'Zed' }),
            member({ userId: '2', displayName: 'Amy' }),
            member({ userId: '3', displayName: 'Cy', role: 'CO_HOST' }),
            member({ userId: '4', displayName: 'Hal', role: 'HOST' }),
            member({ userId: '5', displayName: 'Gone', state: 'LEFT' }),
        ])

        expect(list.map((m) => m.userId)).toEqual(['4', '3', '2', '1'])
    })
})

describe('memberPowers', () => {
    it('lets the host do everything to others', () => {
        expect(memberPowers('HOST', { role: 'MEMBER', isMe: false })).toEqual({
            kick: true,
            makeCoHost: true,
            transfer: true,
        })
        expect(memberPowers('HOST', { role: 'CO_HOST', isMe: false }).makeCoHost).toBe(false)
    })

    it('limits a co-host to kicking members', () => {
        expect(memberPowers('CO_HOST', { role: 'MEMBER', isMe: false })).toEqual({
            kick: true,
            makeCoHost: false,
            transfer: false,
        })
        expect(memberPowers('CO_HOST', { role: 'CO_HOST', isMe: false }).kick).toBe(false)
    })

    it('never acts on yourself, the host, or as a plain member', () => {
        expect(memberPowers('HOST', { role: 'HOST', isMe: true }).kick).toBe(false)
        expect(memberPowers('CO_HOST', { role: 'HOST', isMe: false }).kick).toBe(false)
        expect(memberPowers('MEMBER', { role: 'MEMBER', isMe: false }).kick).toBe(false)
        expect(canManageRole('MEMBER')).toBe(false)
    })
})

describe('invitableFriends', () => {
    it('excludes friends already inside', () => {
        const out = invitableFriends([{ userId: 'a' }, { userId: 'b' }], [{ userId: 'a' }])

        expect(out).toEqual([{ userId: 'b' }])
    })
})

describe('countdown', () => {
    it('ticks down from the site and never goes negative', () => {
        expect(countdownRemaining({ countdownSec: 30 }, 4_200)).toBe(26)
        expect(countdownRemaining({ countdownSec: 3 }, 9_000)).toBe(0)
        expect(countdownRemaining({ countdownSec: null }, 1_000)).toBeNull()
    })

    it('formats a clock and a percent', () => {
        expect(clock(65)).toBe('1:05')
        expect(clock(-3)).toBe('0:00')
        expect(readyPercent({ ready: 1, total: 3 })).toBe(33)
        expect(readyPercent({ ready: 0, total: 0 })).toBe(0)
    })
})

describe('buildCreateRequest', () => {
    it('needs a game', () => {
        expect(buildCreateRequest(BLANK_CREATE).ok).toBe(false)
    })

    it('needs a password for a password party', () => {
        const r = buildCreateRequest({ ...BLANK_CREATE, appId: 1, type: 'PUBLIC_PASSWORD' })

        expect(r.ok).toBe(false)
    })

    it('rejects a bad size, accepts 0 as no limit, and leaves blank to the site', () => {
        expect(buildCreateRequest({ ...BLANK_CREATE, appId: 1, maxUsers: '2.5' }).ok).toBe(false)
        expect(buildCreateRequest({ ...BLANK_CREATE, appId: 1, maxUsers: '-1' }).ok).toBe(false)

        const zero = buildCreateRequest({ ...BLANK_CREATE, appId: 1, maxUsers: '0' })
        const blank = buildCreateRequest({ ...BLANK_CREATE, appId: 1 })

        expect(zero.ok && zero.request.maxUsers).toBe(0)
        expect(blank.ok && 'maxUsers' in blank.request).toBe(false)
    })

    it('trims text, carries the server and sends a password only when it is used', () => {
        const r = buildCreateRequest({
            ...BLANK_CREATE,
            appId: 7,
            serverId: 9,
            name: '  Night raid ',
            password: 'secret',
        })

        expect(r.ok && r.request).toMatchObject({ appId: 7, serverId: 9, name: 'Night raid' })
        expect(r.ok && 'password' in r.request).toBe(false)
    })
})

describe('misc', () => {
    it('clamps reserve minutes, 0 max meaning no limit', () => {
        expect(clampMinutes(1, 60)).toBe(5)
        expect(clampMinutes(500, 60)).toBe(60)
        expect(clampMinutes(500, 0)).toBe(500)
    })

    it('computes kd without dividing by zero', () => {
        expect(kdRatio(10, 4)).toBe('2.50')
        expect(kdRatio(7, 0)).toBe('7.00')
    })

    it('merges pages without duplicates', () => {
        expect(mergePages([{ id: '1' }, { id: '2' }], [{ id: '2' }, { id: '3' }])).toEqual([
            { id: '1' },
            { id: '2' },
            { id: '3' },
        ])
    })
})
