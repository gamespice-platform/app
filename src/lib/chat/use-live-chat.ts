import { useCallback, useMemo, useState } from 'react'

import { liveChatApi } from '~/lib/api/live-chat'
import type {
    LiveChatChannelT,
    LiveChatListResponseT,
    LiveChatMessageT,
    LiveChatTargetT,
} from '~/lib/api/chat-schemas'
import { compareIds, mergeById } from '~/lib/hooks/poll'
import { usePoll, type PollStateT } from '~/lib/hooks/use-poll'
import { LIVE_CHAT_POLL_MS } from './poll-intervals'

type RoomSlotT = {
    id: string
    held: LiveChatMessageT[]
    oldest: string | null
    hasMore: boolean
}

const EMPTY_ROOM = { held: [] as LiveChatMessageT[], oldest: null, hasMore: false }

function foldRoom(
    id: string,
    cur: RoomSlotT,
    page: LiveChatListResponseT,
    edge: 'poll' | 'older'
): RoomSlotT {
    const base = cur.id === id ? cur : { id, ...EMPTY_ROOM }
    const merged = mergeById(base.held, page.messages, byId)

    // The poll only sets the back edge once; paging back moves it.
    if (edge === 'poll' && base.oldest !== null) return { ...base, held: merged }

    return { id, held: merged, oldest: page.oldest ?? base.oldest, hasMore: page.hasMore }
}

const byId = (a: LiveChatMessageT, b: LiveChatMessageT) => compareIds(a.id, b.id)

export type LiveRoomT = PollStateT<unknown> & {
    /** Oldest first. */
    messages: LiveChatMessageT[]
    channel: LiveChatChannelT | null
    hasMore: boolean
    loadOlder: () => Promise<void>
    post: (body: string, replyToId?: string | null) => Promise<void>
}

/**
 * A live-chat room as a subscription: the newest page first, then only
 * messages `after` the newest one held, every `LIVE_CHAT_POLL_MS`. Edits and
 * deletions to older lines arrive on the next full-page refresh, `refresh()`
 * after a post folds your own in at once.
 */
export function useLiveChat(target: LiveChatTargetT | null): LiveRoomT {
    const id = target ? `${target.scope}:${target.partyId ?? ''}:${target.serverId ?? ''}` : ''

    // Tagged with the room they belong to: switching rooms reads as empty.
    const [slot, setSlot] = useState<RoomSlotT>({ id, ...EMPTY_ROOM })
    const [chan, setChan] = useState<{ id: string; value: LiveChatChannelT | null }>()
    const room = slot.id === id ? slot : { id, ...EMPTY_ROOM }
    const { held, oldest, hasMore } = room
    const channel = chan?.id === id ? chan.value : null

    // The target object is usually rebuilt each render; its parts are the identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    const stable = useMemo(() => target, [id])

    const newest = held.length > 0 ? held[held.length - 1]!.id : null

    const fetcher = useCallback(async () => {
        if (!stable) throw new Error('No room')

        return await liveChatApi.history(stable, newest ? { after: newest } : {})
    }, [stable, newest])

    const poll = usePoll(fetcher, {
        intervalMs: LIVE_CHAT_POLL_MS,
        enabled: stable != null,
        key: `live:${id}`,
        onData: (page) => {
            setSlot((cur) => foldRoom(id, cur, page, 'poll'))
        },
    })

    const loadOlder = useCallback(async () => {
        if (!stable || !oldest) return

        const page = await liveChatApi.history(stable, { before: oldest })

        setSlot((cur) => foldRoom(id, cur, page, 'older'))
    }, [stable, oldest, id])

    const refresh = poll.refresh

    const post = useCallback(
        async (body: string, replyToId?: string | null) => {
            if (!stable) return

            const made = await liveChatApi.post(stable, body, replyToId)

            setSlot((cur) => {
                const base = cur.id === id ? cur : { id, ...EMPTY_ROOM }

                return { ...base, held: mergeById(base.held, [made], byId) }
            })
            await refresh()
        },
        [stable, refresh, id]
    )

    // The channel (can I post, and why not) changes rarely; fetch it once per room.
    usePoll(
        async () => {
            if (!stable) return null

            const c = await liveChatApi.channel(stable)

            setChan({ id, value: c })

            return c
        },
        { intervalMs: 60_000, enabled: stable != null, key: `channel:${id}` }
    )

    return { ...poll, messages: held, channel, hasMore, loadOlder, post }
}
