import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
    invoke: (...a: unknown[]): unknown => invoke(...a),
}))

import { IpcError } from '~/lib/ipc'
import { PartyReadyViewSchema, PartyReservationSchema } from './party-schemas'
import { partyApi } from './party'
import { isUnavailable } from './unavailable'

/*
 * The payloads are the shapes website-city's `/api/app/v1/party/*` handlers
 * build (`PartyReadyViewT`, `ReservationRowT`, `ReserveOfferT`), dates as the
 * JSON ISO strings the envelope carries. The client runs end to end against a
 * mocked Tauri `invoke`, so the path, method, body and parse are all exercised.
 */

const ready = {
    stage: 'READY',
    openedAt: '2026-11-22T20:00:00.000Z',
    elapsedSec: 12,
    total: 3,
    ready: 2,
    pct: 66,
    autoPct: 0,
    autoSec: 30,
    countdownSec: null,
    members: [
        {
            id: '1',
            userId: 'clx1',
            name: 'Ashley',
            avatarKey: null,
            linkable: true,
            role: 'HOST',
            ready: true,
            connected: false,
            isMe: true,
        },
    ],
    membersHidden: false,
    me: { isMember: true, ready: true, connected: false, canManage: true },
    connect: {
        serverId: 4821,
        serverName: 'EU #1',
        url: 'steam://connect/1.2.3.4:27015',
        href: null,
        webPlay: false,
    },
    started: false,
}

const booking = {
    id: '90',
    status: 'PENDING',
    private: false,
    startsAt: '2026-11-22T21:00:00.000Z',
    endsAt: '2026-11-22T22:00:00.000Z',
    releasedAt: null,
    note: null,
    partyId: '4471',
    partyName: 'Friday night',
    partyUsers: 3,
    server: { id: 4821, name: 'EU #1', url: 'tmc://x', online: true, curUsers: 2, maxUsers: 16 },
    requestedByName: 'Ashley',
}

describe('party client', () => {
    beforeEach(() => invoke.mockReset())

    it('parses the ready panel and sends ready as a POST body', async () => {
        invoke.mockResolvedValue(ready)

        const view = await partyApi.setReady('4471', false)

        expect(view.me.canManage).toBe(true)
        expect(PartyReadyViewSchema.parse(ready).connect?.serverId).toBe(4821)
        expect(invoke).toHaveBeenCalledWith('api_send', {
            method: 'POST',
            path: '/party/ready',
            body: { id: '4471', ready: false },
            auth: true,
        })
    })

    it('reads a party by id, encoded, and the ready panel under it', async () => {
        invoke.mockResolvedValue(ready)

        await partyApi.readyState('4471')

        expect(invoke).toHaveBeenCalledWith('api_get', {
            path: '/party/4471/ready',
            query: undefined,
            auth: true,
        })
    })

    it('sends invite ids as a list and kick with ban defaulting off', async () => {
        invoke.mockResolvedValueOnce(['11', '12'])
        await expect(partyApi.invite('4471', ['a', 'b'], 'join us')).resolves.toEqual(['11', '12'])

        invoke.mockResolvedValueOnce(null)
        await partyApi.kick('4471', 'clx9')

        expect(invoke).toHaveBeenLastCalledWith('api_send', {
            method: 'POST',
            path: '/party/kick',
            body: { id: '4471', userId: 'clx9', ban: false },
            auth: true,
        })
    })

    it('passes the server as a query on the reserve offer', async () => {
        invoke.mockResolvedValue({
            available: false,
            deny: 'taken',
            allowPublic: true,
            allowPrivate: false,
            maxMinutes: 120,
            opensAt: null,
            freeAt: '2026-11-22T22:00:00.000Z',
        })

        const offer = await partyApi.reserveOffer('4471', 4821)

        expect(offer.deny).toBe('taken')
        expect(invoke).toHaveBeenCalledWith('api_get', {
            path: '/party/4471/reserve/offer',
            query: [['serverId', '4821']],
            auth: true,
        })
    })

    it('parses a booking', async () => {
        invoke.mockResolvedValue(booking)

        expect((await partyApi.reserve({ partyId: '4471', serverId: 4821, minutes: 60 })).status).toBe(
            'PENDING'
        )
        expect(PartyReservationSchema.safeParse({ ...booking, endsAt: 5 }).success).toBe(false)
    })

    it('refuses a response that drifted from the contract', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        invoke.mockResolvedValue({ ...ready, total: 'three' })

        await expect(partyApi.readyState('4471')).rejects.toThrow(/does not understand/)
    })

    it('reports a route the site does not serve as unavailable', () => {
        const notRouted = new IpcError({
            code: 'network',
            message: 'Unexpected response from the server (404 Not Found).',
        })

        expect(isUnavailable(notRouted)).toBe(true)
        expect(isUnavailable(new IpcError({ code: 'network', message: 'Could not reach the server.' }))).toBe(false)
        expect(isUnavailable(new IpcError({ code: 'api', message: 'party.join.deny.full' }))).toBe(false)
    })
})
