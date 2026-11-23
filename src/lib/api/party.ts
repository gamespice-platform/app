import { z } from 'zod'

import { PartyInviteRespondResponse, PartyLeaveResponse, PartyJoinResponse, PartyMineResponse, PartyInvitesResponse, PartySchema } from './contract'
import {
    PartyBrowseResponse,
    PartyInviteSentResponse,
    PartyModerationRequest,
    PartyReadyViewSchema,
    PartyReservationSchema,
    PartyReserveOfferSchema,
    PartySearchSchema,
    PartyStatsRowSchema,
    type PartyBrowseQueryT,
    type PartyCreateRequestT,
    type PartyJoinRequestT,
    type PartyReserveRequestT,
} from './party-schemas'
import { get, send } from './transport'

/**
 * Parties, as the app reaches them. Two groups, and the split is the point:
 *
 * - **REST** (`/api/app/v1/party/*`): the game-client routes. They accept the
 *   device token Rust holds, so these work today.
 * - **Proposed** (`browse`, `search*`, `stats`, `moderate`): the site serves
 *   them over tRPC with a session cookie only, so the app has no way in. The
 *   calls target the routes website-city would add and fail with
 *   `isUnavailable(e) === true` until it does. docs/party-chat-api.md.
 *
 * Every id is a decimal string. A refusal's message is the site's own.
 */

const Null = z.null()

export const partyApi = {
    // ------------------------------------------------------------- Reading
    /** The caller's live party, or null. */
    mine: () => get('/party/mine', PartyMineResponse, undefined, true),

    one: (id: string) => get(`/party/${encodeURIComponent(id)}`, PartySchema, undefined, true),

    invites: () => get('/party/invites', PartyInvitesResponse, undefined, true),

    /** The ready panel: who is ready, the countdown, and how to connect. */
    readyState: (id: string) =>
        get(`/party/${encodeURIComponent(id)}/ready`, PartyReadyViewSchema, undefined, true),

    // ----------------------------------------------------------- Lifecycle
    /** Also saves when `input.id` is a party the caller manages. */
    create: (input: PartyCreateRequestT) =>
        send('POST', '/party/create', PartySchema, input),

    join: (input: PartyJoinRequestT) =>
        send('POST', '/party/join', PartyJoinResponse, input),

    leave: (id: string, silent?: boolean) =>
        send('POST', '/party/leave', PartyLeaveResponse, { id, silent }),

    /** Host only. Ends the party for everyone. */
    end: (id: string) => send('POST', '/party/end', Null, { id }),

    heartbeat: (id: string) => send('POST', '/party/heartbeat', Null, { id }),

    // ------------------------------------------------------------- Invites
    /** The invite ids made; fewer than `userIds` when some were skipped. */
    invite: (id: string, userIds: string[], message?: string) =>
        send('POST', '/party/invite', PartyInviteSentResponse, { id, userIds, message }),

    /** Accepting also JOINS, so capacity and bans still decide. */
    respondToInvite: (inviteId: string, accept: boolean) =>
        send('POST', '/party/invite/respond', PartyInviteRespondResponse, { inviteId, accept }),

    // ------------------------------------------------------------- Members
    kick: (id: string, userId: string, ban = false) =>
        send('POST', '/party/kick', Null, { id, userId, ban }),

    setRole: (id: string, userId: string, role: 'CO_HOST' | 'MEMBER') =>
        send('POST', '/party/role', Null, { id, userId, role }),

    transferHost: (id: string, userId: string) =>
        send('POST', '/party/transfer', Null, { id, userId }),

    // --------------------------------------------------------------- Ready
    /** `ready: false` un-readies, for when the game crashed. */
    setReady: (id: string, ready = true) =>
        send('POST', '/party/ready', PartyReadyViewSchema, { id, ready }),

    connected: (id: string) => send('POST', '/party/connected', Null, { id }),

    /** Manager only: the ready check opens (and picks `serverId` when given). */
    start: (id: string, serverId?: number) =>
        send('POST', '/party/start', PartyReadyViewSchema, { id, serverId }),

    forceStart: (id: string) => send('POST', '/party/force-start', Null, { id }),

    cancelStart: (id: string) => send('POST', '/party/cancel', Null, { id }),

    // ------------------------------------------------------------- Reserve
    reserveOffer: (id: string, serverId: number) =>
        get(
            `/party/${encodeURIComponent(id)}/reserve/offer`,
            PartyReserveOfferSchema,
            { serverId },
            true
        ),

    reserve: (input: PartyReserveRequestT) =>
        send('POST', '/party/reserve', PartyReservationSchema, input),

    cancelReservation: (reservationId: string) =>
        send('POST', '/party/reserve/cancel', PartyReservationSchema, { reservationId }),

    // -------------------------------------- Proposed: no app route on the site
    browse: (query: PartyBrowseQueryT = {}) =>
        get('/party/browse', PartyBrowseResponse, query, true),

    /** Matchmaking: start looking, then poll `searchState`. */
    searchStart: (id: string) => send('POST', '/party/search/start', PartySearchSchema, { id }),

    searchState: (id: string) =>
        get('/party/search', PartySearchSchema.nullable(), { id }, true),

    searchCancel: (id: string) => send('POST', '/party/search/cancel', Null, { id }),

    /** The caller's lifetime party totals. */
    stats: () => get('/party/stats/me', PartyStatsRowSchema, undefined, true),

    /** Staff only. A member token is refused by role, not by route. */
    moderate: (input: z.infer<typeof PartyModerationRequest>) =>
        send('POST', '/party/moderate', Null, input),
}

export type PartyApiT = typeof partyApi
