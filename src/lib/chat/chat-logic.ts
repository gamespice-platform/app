import type {
    ChatMessageT,
    ChatThreadT,
    LiveChatChannelT,
    LiveChatMessageT,
} from '~/lib/api/chat-schemas'

/** Pure rules for the chat screens, apart from React so they are tested alone. */

/** Whether a key press in the composer sends: Enter alone; Shift+Enter is a newline; never mid-IME. */
export function sendsOnKey(e: {
    key: string
    shiftKey: boolean
    ctrlKey?: boolean
    metaKey?: boolean
    altKey?: boolean
    isComposing?: boolean
}): boolean {
    if (e.key !== 'Enter' || e.isComposing) return false
    if (e.altKey) return false

    return !e.shiftKey
}

/** The text to send, or null when there is nothing worth sending. */
export function sendable(text: string): string | null {
    const trimmed = text.trim()

    return trimmed.length > 0 ? trimmed : null
}

/** "3", or "" for none: the badge draws nothing at zero. No ceiling: the count is the count. */
export function unreadLabel(count: number): string {
    return count > 0 ? String(count) : ''
}

/** What a thread is called: its title, else the other people, else a plain fallback. */
export function threadTitle(t: Pick<ChatThreadT, 'title' | 'others' | 'isGroup'>): string {
    if (t.title?.trim()) return t.title.trim()

    const names = t.others.map((u) => u.name ?? u.username ?? 'Someone')

    if (names.length > 0) return names.join(', ')

    return t.isGroup ? 'Group chat' : 'Conversation'
}

/** Newest activity first; unread threads do not jump the queue, the site does not either. */
export function sortThreads(threads: readonly ChatThreadT[]): ChatThreadT[] {
    return [...threads].sort((a, b) =>
        a.lastMessageAt < b.lastMessageAt ? 1 : a.lastMessageAt > b.lastMessageAt ? -1 : 0
    )
}

export type ChatRowT =
    | { kind: 'day'; key: string; label: string }
    | { kind: 'system'; key: string; message: ChatMessageT }
    | { kind: 'message'; key: string; message: ChatMessageT; head: boolean }

const dayKey = (iso: string) => iso.slice(0, 10)

/** How far apart two lines from one sender may be and still share a header. */
export const GROUP_GAP_MS = 5 * 60_000

/**
 * Oldest-first messages as rows: a day divider where the date changes, system
 * lines on their own, and `head` true when a sender's run starts (so the name
 * and time show once per run).
 */
export function chatRows(
    messages: readonly ChatMessageT[],
    dayLabel: (iso: string) => string = dayKey
): ChatRowT[] {
    const rows: ChatRowT[] = []
    let prev: ChatMessageT | null = null
    let lastDay = ''

    for (const m of messages) {
        if (lastDay !== dayKey(m.createdAt)) {
            rows.push({ kind: 'day', key: `day-${dayKey(m.createdAt)}`, label: dayLabel(m.createdAt) })
            lastDay = dayKey(m.createdAt)
        }

        if (m.systemKey) {
            rows.push({ kind: 'system', key: m.id, message: m })
            prev = null
            continue
        }

        const sameRun =
            prev !== null &&
            !prev.systemKey &&
            dayKey(prev.createdAt) === dayKey(m.createdAt) &&
            prev.mine === m.mine &&
            prev.sender?.id === m.sender?.id &&
            Date.parse(m.createdAt) - Date.parse(prev.createdAt) < GROUP_GAP_MS

        rows.push({ kind: 'message', key: m.id, message: m, head: !sameRun })
        prev = m
    }

    return rows
}

/** The English for a system line. */
export function systemText(m: Pick<ChatMessageT, 'systemKey' | 'systemArgs'>): string {
    const who = m.systemArgs?.name ?? m.systemArgs?.user ?? 'Someone'

    switch (m.systemKey) {
        case 'created':
            return `${who} started the conversation`
        case 'added':
            return `${who} was added`
        case 'removed':
            return `${who} was removed`
        case 'left':
            return `${who} left`
        case 'renamed':
            return `Renamed to ${m.systemArgs?.title ?? m.systemArgs?.name ?? 'a new name'}`
        default:
            return 'Conversation updated'
    }
}

export type LineStateT =
    | { kind: 'normal' }
    | { kind: 'deleted' }
    | { kind: 'hidden' }
    | { kind: 'muted' }

/**
 * What to draw for a live-chat line. A removed line is a stub, a moderator-hidden
 * one says so, and a muted author's line is shown to its author only by the site
 * (`muted`), so it carries a note.
 */
export function lineState(m: Pick<LiveChatMessageT, 'deleted' | 'hidden' | 'muted'>): LineStateT {
    if (m.deleted) return { kind: 'deleted' }
    if (m.hidden) return { kind: 'hidden' }
    if (m.muted) return { kind: 'muted' }

    return { kind: 'normal' }
}

/** The member-facing reason posting is refused, from the channel the site reports. */
export function postBlockReason(channel: LiveChatChannelT | null): string | null {
    if (!channel) return null
    if (channel.locked) return 'This chat is locked.'
    if (channel.muted) return 'You are muted in this chat.'
    if (!channel.canPost) {
        const key = channel.denyKey ?? ''

        if (/verif/i.test(key)) return 'Verify your account to post here.'
        if (/ban/i.test(key)) return 'You are banned from this chat.'
        if (/member|join/i.test(key)) return 'Only members of this room can post.'

        return 'You cannot post in this chat.'
    }

    return null
}

/**
 * Seconds to wait if an error reads as the site's rate limit, else null.
 * The site's message names the wait ("Try again in 12 seconds"); with none given
 * the caller should still show the limit state, so this returns 0 for "limited,
 * unknown wait".
 */
export function rateLimitWait(message: string): number | null {
    if (!/rate.?limit|too many|slow down|too fast|try again in/i.test(message)) return null

    const m = /(\d+)\s*(ms|milliseconds?|s|sec|seconds?|m|min|minutes?)\b/i.exec(message)

    if (!m) return 0

    const n = Number(m[1])
    const unit = (m[2] ?? 's').toLowerCase()

    if (unit.startsWith('ms') || unit.startsWith('milli')) return Math.ceil(n / 1000)
    if (unit.startsWith('m')) return n * 60

    return n
}

/** Quick reactions offered on a line; any emoji already on it stays toggleable. */
export const QUICK_REACTIONS = ['👍', '❤️', '😂', '🎉', '😮'] as const

/** "14:05" in the viewer's locale, or '' for an unparseable stamp. */
export function clockTime(iso: string): string {
    const t = Date.parse(iso)

    return Number.isNaN(t)
        ? ''
        : new Date(t).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
}
