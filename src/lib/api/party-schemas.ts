import { z } from 'zod'

/**
 * Party payloads the contract mirror does not carry yet.
 *
 * `contract.ts` is a verbatim copy of website-city's contract and is never
 * edited by hand, and its "Social" section stops at `mine`, `invites`, `join`,
 * `invite/respond` and `leave`. The rest of the game-client party routes under
 * `/api/app/v1/party/*` take and return the shapes below, which are copied from
 * the site's own types (`~/types/party/*`). When the contract grows them, delete
 * the duplicate here and import it from `./contract`.
 *
 * Ids are decimal strings, dates are ISO strings (the REST envelope is JSON),
 * and enum-like fields stay strings so a value added on the site is not a parse
 * failure on an older app. The `*Vals` lists are for building forms only.
 */

export const PartyIdSchema = z.string().min(1).max(20).regex(/^\d+$/)

export const PartyTypeVals = ['PUBLIC', 'PUBLIC_PASSWORD', 'FRIENDS', 'PRIVATE'] as const
export const PartyStageVals = ['LOBBY', 'SEARCHING', 'READY', 'PLAYING'] as const
export const PartyMemberRoleVals = ['HOST', 'CO_HOST', 'MEMBER'] as const

// ------------------------------------------------------------------ Requests

/** `POST party/create` — also saves when `id` names a party the caller manages. */
export const PartyCreateRequest = z.object({
    id: PartyIdSchema.optional(),
    appId: z.number().int().positive().optional(),
    categoryId: z.number().int().positive().nullish(),
    serverId: z.number().int().positive().nullish(),
    name: z.string().nullish(),
    description: z.string().nullish(),
    type: z.enum(PartyTypeVals).optional(),
    password: z.string().nullish(),
    maxUsers: z.number().int().optional(),
    friendsCanJoin: z.boolean().optional(),
    chatEnabled: z.boolean().optional(),
    chatPublic: z.boolean().optional(),
    showUsers: z.boolean().optional(),
    nsfw: z.boolean().optional(),
    plannedEndTime: z.string().nullish(),
    joinAfterStartPublic: z.boolean().optional(),
    joinAfterStartFriends: z.boolean().optional(),
    /** 0 = everybody must be ready. */
    readyAutoPct: z.number().int().optional(),
    readyAutoSec: z.number().int().optional(),
    mediaEnabled: z.boolean().optional(),
    mediaMembersUpload: z.boolean().optional(),
})
export type PartyCreateRequestT = z.infer<typeof PartyCreateRequest>

/** `POST party/join`. A password for `PUBLIC_PASSWORD`, a token from an invite link. */
export const PartyJoinRequest = z.object({
    id: PartyIdSchema,
    password: z.string().optional(),
    token: z.string().optional(),
})
export type PartyJoinRequestT = z.infer<typeof PartyJoinRequest>

/** `POST party/invite` — the site skips anybody it cannot invite. */
export const PartyInviteRequest = z.object({
    id: PartyIdSchema,
    userIds: z.array(z.string().min(1)).min(1),
    message: z.string().nullish(),
})

/** `POST party/kick`. */
export const PartyKickRequest = z.object({
    id: PartyIdSchema,
    userId: z.string().min(1),
    ban: z.boolean().optional(),
})

/** `POST party/role` — `HOST` is a transfer, not a role. */
export const PartyRoleRequest = z.object({
    id: PartyIdSchema,
    userId: z.string().min(1),
    role: z.enum(['CO_HOST', 'MEMBER']),
})

/** `POST party/transfer`. */
export const PartyTransferRequest = z.object({
    id: PartyIdSchema,
    userId: z.string().min(1),
})

/** `POST party/ready`. A false un-readies. */
export const PartyReadyRequest = z.object({
    id: PartyIdSchema,
    ready: z.boolean().optional(),
})

/** `POST party/reserve`. */
export const PartyReserveRequest = z.object({
    partyId: PartyIdSchema,
    serverId: z.number().int().positive(),
    minutes: z.number().int().min(5),
    private: z.boolean().optional(),
    note: z.string().nullish(),
})
export type PartyReserveRequestT = z.infer<typeof PartyReserveRequest>

// ----------------------------------------------------------------- Responses

/** `POST party/invite` — the invites made; shorter than `userIds` when some were skipped. */
export const PartyInviteSentResponse = z.array(z.string())

export const PartyReadyMemberSchema = z.object({
    id: z.string(),
    userId: z.string().nullable(),
    name: z.string().nullable(),
    avatarKey: z.string().nullable(),
    linkable: z.boolean(),
    role: z.string(),
    ready: z.boolean(),
    connected: z.boolean(),
    isMe: z.boolean(),
})

export const PartyReadyConnectSchema = z.object({
    serverId: z.number().int(),
    serverName: z.string().nullable(),
    /** The app's connect template, resolved. Null when it cannot be. */
    url: z.string().nullable(),
    href: z.string().nullable(),
    webPlay: z.boolean(),
})

/** `GET party/{id}/ready` and `POST party/ready`, `party/start`. */
export const PartyReadyViewSchema = z.object({
    stage: z.string(),
    openedAt: z.string().nullable(),
    /** Server-vouched; tick locally from this. */
    elapsedSec: z.number(),
    total: z.number().int(),
    ready: z.number().int(),
    pct: z.number(),
    autoPct: z.number(),
    autoSec: z.number(),
    countdownSec: z.number().nullable(),
    members: z.array(PartyReadyMemberSchema),
    /** The roster was withheld from this viewer rather than being empty. */
    membersHidden: z.boolean(),
    me: z.object({
        isMember: z.boolean(),
        ready: z.boolean(),
        connected: z.boolean(),
        canManage: z.boolean(),
    }),
    connect: PartyReadyConnectSchema.nullable(),
    started: z.boolean(),
})
export type PartyReadyViewT = z.infer<typeof PartyReadyViewSchema>

/** `GET party/{id}/reserve/offer?serverId=`. */
export const PartyReserveOfferSchema = z.object({
    available: z.boolean(),
    deny: z.string().nullable(),
    allowPublic: z.boolean(),
    allowPrivate: z.boolean(),
    maxMinutes: z.number(),
    opensAt: z.string().nullable(),
    freeAt: z.string().nullable(),
})
export type PartyReserveOfferT = z.infer<typeof PartyReserveOfferSchema>

/** One server booking; the answer of `party/reserve` and `party/reserve/cancel`. */
export const PartyReservationSchema = z.object({
    id: z.string(),
    status: z.string(),
    private: z.boolean(),
    startsAt: z.string(),
    endsAt: z.string(),
    releasedAt: z.string().nullable(),
    note: z.string().nullable(),
    partyId: z.string(),
    partyName: z.string().nullable(),
    partyUsers: z.number().int(),
    server: z.object({
        id: z.number().int(),
        name: z.string().nullable(),
        url: z.string(),
        online: z.boolean(),
        curUsers: z.number(),
        maxUsers: z.number(),
    }),
    requestedByName: z.string().nullable(),
})
export type PartyReservationT = z.infer<typeof PartyReservationSchema>

// ------------------------------------------------------------ Not on REST yet
/*
 * Everything below has NO `/api/app/v1` route today, only a tRPC procedure the
 * session cookie guards (see docs/party-chat-api.md). The shapes are proposed
 * from the procedures' own types so the client is ready the day a route lands;
 * until then the call fails as "unavailable" (`isUnavailable` in `./unavailable`).
 */

export const PartyBrowseQuery = z.object({
    appId: z.number().int().positive().optional(),
    q: z.string().optional(),
    cursor: z.string().nullish(),
    limit: z.number().int().min(1).max(100).optional(),
})
export type PartyBrowseQueryT = z.infer<typeof PartyBrowseQuery>

export const PartyBrowseRowSchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    type: z.string(),
    stage: z.string(),
    appId: z.number().int(),
    appName: z.string().nullable(),
    users: z.number().int(),
    maxUsers: z.number().int(),
    hostName: z.string().nullable(),
    mapName: z.string().nullable(),
    gameMode: z.string().nullable(),
})
export const PartyBrowseResponse = z.object({
    items: z.array(PartyBrowseRowSchema),
    nextCursor: z.string().nullable(),
})
export type PartyBrowseResponseT = z.infer<typeof PartyBrowseResponse>

/** The matchmaker (`party.search.*`): start, poll, cancel. */
export const PartySearchSchema = z.object({
    id: z.string(),
    state: z.string(),
    partyId: z.string().nullable(),
})
export type PartySearchT = z.infer<typeof PartySearchSchema>

export const PartyStatsRowSchema = z.object({
    kills: z.number(),
    deaths: z.number(),
    assist: z.number(),
    score: z.number(),
    parties: z.number().int(),
})
export type PartyStatsT = z.infer<typeof PartyStatsRowSchema>

/** `party.moderation.*` — staff only; a member token is refused by role. */
export const PartyModerationRequest = z.object({
    id: PartyIdSchema,
    hidden: z.boolean().optional(),
    nsfw: z.boolean().optional(),
    chatLocked: z.boolean().optional(),
    end: z.boolean().optional(),
})
