import type { AppPartyMemberT } from '~/lib/api/contract'
import type { PartyCreateRequestT, PartyReadyViewT } from '~/lib/api/party-schemas'

/**
 * Pure party rules the screens share: how a role or stage reads, who may do
 * what, how the create form becomes a request, and the ready countdown. Kept
 * out of the components so they are testable and so every screen agrees.
 */

/** How often the party page tells the site we are still here (the site expects ~10 s). */
export const PARTY_HEARTBEAT_MS = 10_000

const ROLE_LABELS: Record<string, string> = {
    HOST: 'Host',
    CO_HOST: 'Co-host',
    MEMBER: 'Member',
}

const STAGE_LABELS: Record<string, string> = {
    LOBBY: 'Lobby',
    SEARCHING: 'Searching',
    READY: 'Ready check',
    PLAYING: 'Playing',
}

export const PARTY_TYPE_LABELS: Record<string, string> = {
    PUBLIC: 'Public',
    PUBLIC_PASSWORD: 'Public, password',
    FRIENDS: 'Friends only',
    PRIVATE: 'Invite only',
}

/** A value the site added after this app shipped is shown as written, not hidden. */
function titleCase(raw: string): string {
    const text = raw.replace(/_/g, ' ').toLowerCase()

    return text.charAt(0).toUpperCase() + text.slice(1)
}

export const roleLabel = (role: string): string => ROLE_LABELS[role] ?? titleCase(role)
export const stageLabel = (stage: string): string => STAGE_LABELS[stage] ?? titleCase(stage)
export const typeLabel = (type: string): string => PARTY_TYPE_LABELS[type] ?? titleCase(type)

/** Hosts and co-hosts run the party: start, kick, invite, reserve. */
export const canManageRole = (role: string): boolean => role === 'HOST' || role === 'CO_HOST'

const ROLE_RANK: Record<string, number> = { HOST: 0, CO_HOST: 1 }

export const memberName = (m: { displayName: string; gameName: string | null }): string =>
    m.displayName || m.gameName || 'Player'

/** The people actually in the party now, host first, then co-hosts, then by name. */
export function joinedMembers(members: AppPartyMemberT[]): AppPartyMemberT[] {
    return members
        .filter((m) => m.state === 'JOINED' && m.leftAt == null)
        .sort(
            (a, b) =>
                (ROLE_RANK[a.role] ?? 2) - (ROLE_RANK[b.role] ?? 2) ||
                memberName(a).localeCompare(memberName(b))
        )
}

/**
 * What the viewer may do to `target`. The host moves anyone; a co-host only
 * manages plain members; nobody acts on themselves or on the host.
 */
export function memberPowers(
    myRole: string,
    target: { role: string; isMe: boolean }
): { kick: boolean; makeCoHost: boolean; transfer: boolean } {
    const none = { kick: false, makeCoHost: false, transfer: false }

    if (target.isMe || target.role === 'HOST') return none

    if (myRole === 'HOST')
        return { kick: true, makeCoHost: target.role === 'MEMBER', transfer: true }

    if (myRole === 'CO_HOST' && target.role === 'MEMBER')
        return { kick: true, makeCoHost: false, transfer: false }

    return none
}

/** Friends who can still be invited: not already in the party. */
export function invitableFriends<T extends { userId: string }>(
    friends: T[],
    members: { userId: string }[]
): T[] {
    const inside = new Set(members.map((m) => m.userId))

    return friends.filter((f) => !inside.has(f.userId))
}

/**
 * Seconds left on the ready countdown, ticking from the site's own figure (the
 * server vouches for it; the local clock only advances it). Null when the party
 * has no countdown. Never negative.
 */
export function countdownRemaining(
    view: Pick<PartyReadyViewT, 'countdownSec'>,
    sinceAnswerMs: number
): number | null {
    if (view.countdownSec == null) return null

    return Math.max(0, Math.ceil(view.countdownSec - sinceAnswerMs / 1000))
}

/** 0..100, whole percent of the party that is ready. */
export function readyPercent(view: Pick<PartyReadyViewT, 'ready' | 'total'>): number {
    return view.total > 0 ? Math.round((view.ready / view.total) * 100) : 0
}

/** `m:ss`, for a countdown. */
export function clock(seconds: number): string {
    const s = Math.max(0, Math.floor(seconds))

    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

// ------------------------------------------------------------------- Create

export type CreateFormT = {
    appId: number | null
    serverId: number | null
    name: string
    description: string
    type: string
    password: string
    /** Blank = the site's default; 0 = no limit. */
    maxUsers: string
    chatEnabled: boolean
    friendsCanJoin: boolean
    showUsers: boolean
}

export const BLANK_CREATE: CreateFormT = {
    appId: null,
    serverId: null,
    name: '',
    description: '',
    type: 'PUBLIC',
    password: '',
    maxUsers: '',
    chatEnabled: true,
    friendsCanJoin: true,
    showUsers: true,
}

/** The form as a request, or the sentence that says what to fix. */
export function buildCreateRequest(
    form: CreateFormT
): { ok: true; request: PartyCreateRequestT } | { ok: false; error: string } {
    if (form.appId == null) return { ok: false, error: 'Pick the game this party plays.' }

    if (form.type === 'PUBLIC_PASSWORD' && form.password.trim() === '')
        return { ok: false, error: 'A password party needs a password.' }

    const max = form.maxUsers.trim()
    let maxUsers: number | undefined

    if (max !== '') {
        const n = Number(max)

        if (!Number.isInteger(n) || n < 0)
            return { ok: false, error: 'Players must be a whole number (0 for no limit).' }

        maxUsers = n
    }

    const request: PartyCreateRequestT = {
        appId: form.appId,
        type: form.type as PartyCreateRequestT['type'],
        chatEnabled: form.chatEnabled,
        friendsCanJoin: form.friendsCanJoin,
        showUsers: form.showUsers,
    }

    if (form.serverId != null) request.serverId = form.serverId
    if (form.name.trim() !== '') request.name = form.name.trim()
    if (form.description.trim() !== '') request.description = form.description.trim()
    if (form.type === 'PUBLIC_PASSWORD') request.password = form.password
    if (maxUsers !== undefined) request.maxUsers = maxUsers

    return { ok: true, request }
}

// ------------------------------------------------------------------ Reserve

/** Minutes the offer allows, clamped; `maxMinutes` 0 means the site sets no limit. */
export function clampMinutes(minutes: number, maxMinutes: number): number {
    const floor = Math.max(5, Math.floor(minutes) || 5)

    return maxMinutes > 0 ? Math.min(floor, Math.floor(maxMinutes)) : floor
}

// -------------------------------------------------------------------- Stats

/** Kills per death; a death-less record is its kills, not infinity. */
export function kdRatio(kills: number, deaths: number): string {
    return (deaths > 0 ? kills / deaths : kills).toFixed(2)
}

/** Appends a browse page, dropping rows already shown (a poll can overlap pages). */
export function mergePages<T extends { id: string }>(shown: T[], next: T[]): T[] {
    const seen = new Set(shown.map((r) => r.id))

    return [...shown, ...next.filter((r) => !seen.has(r.id))]
}

/**
 * How a ready-check connect link is opened. `https` goes to the system browser
 * (`openUrl`, scoped to https); every other scheme (`steam://` and kin) is
 * handed to Rust by server id, which checks it against `LAUNCH_SCHEMES`.
 */
export function connectRoute(href: string): 'web' | 'launch' {
    return /^https:\/\//i.test(href.trim()) ? 'web' : 'launch'
}

/**
 * The viewer's role in the party. Prefers the ready check's own `isMe` row,
 * then the party roster by the signed-in user id (a hidden roster leaves the
 * ready view without members, but the party payload still lists the host), and
 * only then guesses from `canManage`.
 */
export function myPartyRole(
    userId: string | null | undefined,
    partyMembers: Pick<AppPartyMemberT, 'userId' | 'role'>[],
    readyMembers: { isMe: boolean; role: string }[] | undefined,
    canManage: boolean | undefined
): string {
    const fromReady = readyMembers?.find((m) => m.isMe)

    if (fromReady) return fromReady.role

    const fromParty = userId ? partyMembers.find((m) => m.userId === userId) : undefined

    if (fromParty) return fromParty.role

    return canManage ? 'CO_HOST' : 'MEMBER'
}

export type ModerationFlagsT = { hidden: boolean; nsfw: boolean; chatLocked: boolean }

/** A moderation request carrying only the flags the staffer actually changed. */
export function moderationRequest(
    id: string,
    flags: ModerationFlagsT,
    touched: ReadonlySet<keyof ModerationFlagsT>,
    end: boolean
) {
    const req: { id: string; hidden?: boolean; nsfw?: boolean; chatLocked?: boolean; end?: boolean } = {
        id: id.trim(),
    }

    for (const key of ['hidden', 'nsfw', 'chatLocked'] as const)
        if (touched.has(key)) req[key] = flags[key]

    if (end) req.end = true

    return req
}
