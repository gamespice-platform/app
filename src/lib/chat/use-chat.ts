import { useCallback, useState } from 'react'

import { chatApi } from '~/lib/api/chat'
import type {
    ChatMessageT,
    ChatThreadsResponseT,
    ChatThreadsQueryT,
} from '~/lib/api/chat-schemas'
import { compareIds, mergeById } from '~/lib/hooks/poll'
import { usePoll, type PollStateT } from '~/lib/hooks/use-poll'
import { CHAT_ACTIVE_POLL_MS, CHAT_BADGE_POLL_MS } from './poll-intervals'

/** Unread messages across every thread, for the badge. */
export function useChatUnread(enabled = true): PollStateT<number> {
    return usePoll(() => chatApi.unreadCount(), {
        intervalMs: CHAT_BADGE_POLL_MS,
        enabled,
        key: 'unread',
    })
}

/** The first page of threads, kept fresh. Page further with `chatApi.threads({ cursor })`. */
export function useChatThreads(
    query: ChatThreadsQueryT = {},
    enabled = true
): PollStateT<ChatThreadsResponseT> {
    const key = JSON.stringify(query)

    return usePoll(() => chatApi.threads(query), {
        intervalMs: CHAT_BADGE_POLL_MS,
        enabled,
        key: `threads:${key}`,
    })
}

const EMPTY: ChatMessageT[] = []

type ChatSlotT = {
    chatId: string | null
    held: ChatMessageT[]
    cursor: string | null | undefined
}

const byTime = (a: ChatMessageT, b: ChatMessageT) =>
    a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : compareIds(a.id, b.id)

function foldChat(
    chatId: string | null,
    cur: ChatSlotT,
    items: ChatMessageT[],
    next: string | null,
    first: boolean
): ChatSlotT {
    const base = cur.chatId === chatId ? cur : { chatId, held: EMPTY, cursor: undefined }

    return {
        chatId,
        held: mergeById(base.held, items, byTime),
        // The poll keeps a back edge it already has; paging back moves it.
        cursor: first && base.cursor !== undefined ? base.cursor : next,
    }
}

export type ChatConversationT = PollStateT<unknown> & {
    /** Oldest first, ready to render top to bottom. */
    messages: ChatMessageT[]
    /** False once `loadOlder` has reached the start. */
    hasOlder: boolean
    loadOlder: () => Promise<void>
    send: (content: string, fileIds?: string[]) => Promise<void>
}

/**
 * One open conversation. The newest page is polled and merged by id, so an
 * edit or a deletion replaces the row it changes; older pages stay put.
 * Opening it marks it read, and each poll that brings something new does again.
 */
export function useChatConversation(chatId: string | null): ChatConversationT {
    // Tagged with the chat they belong to: switching chats reads as empty.
    const [slot, setSlot] = useState<ChatSlotT>({ chatId, held: EMPTY, cursor: undefined })
    const mine = slot.chatId === chatId
    const held = mine ? slot.held : EMPTY
    const cursor = mine ? slot.cursor : undefined

    const poll = usePoll(() => chatApi.history(chatId ?? ''), {
        intervalMs: CHAT_ACTIVE_POLL_MS,
        enabled: chatId != null,
        key: `chat:${chatId ?? ''}`,
        onData: (page) => {
            setSlot((cur) => foldChat(chatId, cur, page.items, page.nextCursor, true))

            if (chatId && page.items.some((m) => !m.mine))
                void chatApi.read(chatId).catch(() => undefined)
        },
    })

    const loadOlder = useCallback(async () => {
        if (!chatId || !cursor) return

        const page = await chatApi.history(chatId, cursor)

        setSlot((cur) => foldChat(chatId, cur, page.items, page.nextCursor, false))
    }, [chatId, cursor])

    const refresh = poll.refresh

    const send = useCallback(
        async (content: string, fileIds?: string[]) => {
            if (!chatId) return

            await chatApi.send({ chatId, content, fileIds })
            await refresh()
        },
        [chatId, refresh]
    )

    return { ...poll, messages: held, hasOlder: cursor != null, loadOlder, send }
}
